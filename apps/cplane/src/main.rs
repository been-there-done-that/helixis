use cplane::app_router;
use cplane::handlers::AppState;
use persistence::db;
use persistence::repositories::task_repo::PostgresTaskRepository;
use std::env;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let _ = dotenvy::dotenv();

    let db_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/helixis".into());
        
    let pool = db::create_pool(&db_url)
        .await
        .expect("Failed to create postgres pool");
        
    let task_repo = Arc::new(PostgresTaskRepository::new(pool));
    let state = Arc::new(AppState { task_repo });
    let app = app_router(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}
