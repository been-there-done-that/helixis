use application::ports::repositories::TaskRepository;
use cplane::app_router;
use cplane::handlers::AppState;
use persistence::db;
use persistence::repositories::executor_repo::PostgresExecutorRepository;
use persistence::repositories::task_repo::PostgresTaskRepository;
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

    let pool = db::create_pool(&db_url)
        .await
        .expect("Failed to create postgres pool");

    let task_repo = Arc::new(PostgresTaskRepository::new(pool.clone()));
    let executor_repo = Arc::new(PostgresExecutorRepository::new(pool));

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
        executor_repo,
    });
    let app = app_router(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
