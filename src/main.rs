use axum::Router;
use sqlx::{Pool, Postgres, postgres::PgPoolOptions};
use std::time::Duration;

#[derive(Clone)]
struct AppState {
    db: Pool<Postgres>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")?;
    let db = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&db).await?;

    let state = AppState { db };

    let app = Router::new().with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
