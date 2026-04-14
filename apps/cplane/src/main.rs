use application::ports::repositories::TaskRepository;
use cplane::app_router;
use cplane::handlers::AppState;
use cplane::live_logs::LiveLogHub;
use cplane::storage::build_s3_store;
use persistence::db;
use persistence::repositories::artifact::PostgresArtifactRepository;
use persistence::repositories::executor::PostgresExecutorRepository;
use persistence::repositories::rate_limit::PostgresRateLimitRepository;
use persistence::repositories::secret::PostgresSecretRepository;
use persistence::repositories::task::PostgresTaskRepository;
use std::env;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::time::{Duration, interval};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let _ = dotenvy::dotenv();

    let db_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/helixis".into());
    let s3_endpoint =
        env::var("HELIXIS_S3_ENDPOINT").unwrap_or_else(|_| "http://localhost:9000".into());
    let s3_access_key = env::var("HELIXIS_S3_ACCESS_KEY").unwrap_or_else(|_| "minioadmin".into());
    let s3_secret_key = env::var("HELIXIS_S3_SECRET_KEY").unwrap_or_else(|_| "minioadmin".into());
    let output_bucket = env::var("HELIXIS_OUTPUT_BUCKET").unwrap_or_else(|_| "task-outputs".into());

    let pool = db::create_pool(&db_url)
        .await
        .expect("Failed to create postgres pool");

    let task_repo = Arc::new(PostgresTaskRepository::new(pool.clone()));
    let artifact_repo = Arc::new(PostgresArtifactRepository::new(pool.clone()));
    let executor_repo = Arc::new(PostgresExecutorRepository::new(pool.clone()));
    let rate_limit_repo = Arc::new(PostgresRateLimitRepository::new(pool.clone()));
    let secret_key = env::var("HELIXIS_SECRETS_KEY")
        .unwrap_or_else(|_| "dev-only-insecure-key-change-me".to_string());
    let secret_repo = Arc::new(PostgresSecretRepository::new(pool, &secret_key));
    let output_store = build_s3_store(&s3_endpoint, &s3_access_key, &s3_secret_key, &output_bucket);

    let reconciler_repo = Arc::clone(&task_repo);
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(5));
        loop {
            ticker.tick().await;
            match reconciler_repo.requeue_expired_leases().await {
                Ok(0) => {}
                Ok(count) => tracing::info!("Requeued {count} task(s) from expired leases"),
                Err(err) => tracing::error!("Lease reconciliation failed: {:?}", err),
            }
        }
    });

    let state = Arc::new(AppState {
        task_repo,
        artifact_repo,
        executor_repo,
        rate_limit_repo,
        secret_repo,
        output_store,
        log_hub: Arc::new(LiveLogHub::new()),
    });
    let app = app_router(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
