use payment_server::{provider, webhooks};
use sqlx::postgres::PgPoolOptions;
use std::{net::SocketAddr, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let database_url = std::env::var("DATABASE_URL")?;
    let db = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await?;
    sqlx::migrate!("./provider_migrations").run(&db).await?;
    let address: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3001".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    let secret = std::env::var("DEMO_WEBHOOK_SECRET").ok();
    let demo = webhooks::demo_router(db.clone(), secret.as_deref())
        .map_err(|error| std::io::Error::other(error.message))?;
    axum::serve(listener, provider::router(db).merge(demo)).await?;
    Ok(())
}
