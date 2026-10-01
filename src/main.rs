use payment_server::{
    AppState, auth, error::AppError, payments::Worker, router_with_webhooks, webhooks,
};
use serde_json::{Value, json};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{net::SocketAddr, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if !arguments.is_empty() && !valid_admin_arguments(&arguments) {
        eprintln!(
            "Usage: payment-server [admin create-business <name> | admin create-key <business-id> | admin list-keys <business-id> | admin revoke-key <key-id>]"
        );
        std::process::exit(2);
    }
    let database_url = std::env::var("DATABASE_URL")?;
    let db = PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    if !arguments.is_empty() {
        match admin(&db, &arguments[1..]).await {
            Ok(output) => println!("{}", serde_json::to_string(&output)?),
            Err(error) => {
                eprintln!(
                    "{}",
                    json!({ "error": { "code": error.code, "message": error.message } })
                );
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let allowed_urls = std::env::var("WEBHOOK_ALLOWED_URLS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .collect();
    let webhook_config =
        webhooks::WebhookConfig::new(allowed_urls, std::env::var("DEMO_WEBHOOK_URL").ok())
            .map_err(|error| std::io::Error::other(error.message))?;
    let webhook_client =
        webhooks::delivery_client().map_err(|error| std::io::Error::other(error.message))?;
    let webhook_db = db.clone();
    let address: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    let psp_url = std::env::var("PSP_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into());
    let worker =
        Worker::new(db.clone(), &psp_url).map_err(|error| std::io::Error::other(error.message))?;
    tokio::try_join!(
        async {
            axum::serve(
                listener,
                router_with_webhooks(AppState { db }, webhook_config),
            )
            .await
        },
        async {
            worker.run().await;
            Err::<(), _>(std::io::Error::other("Payment worker unexpectedly stopped"))
        },
        async {
            webhooks::run(webhook_db, webhook_client)
                .await
                .map_err(|error| std::io::Error::other(error.message))?;
            Err::<(), _>(std::io::Error::other("Webhook worker unexpectedly stopped"))
        }
    )?;
    Ok(())
}

fn valid_admin_arguments(arguments: &[String]) -> bool {
    arguments.len() == 3
        && arguments[0] == "admin"
        && matches!(
            arguments[1].as_str(),
            "create-business" | "create-key" | "list-keys" | "revoke-key"
        )
}

async fn admin(db: &PgPool, arguments: &[String]) -> Result<Value, AppError> {
    if arguments[0] == "create-business" {
        return serde_json::to_value(auth::create_business(db, &arguments[1]).await?)
            .map_err(AppError::internal);
    }
    let id = arguments[1]
        .parse::<i64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| AppError::invalid_input("ID must be a positive integer"))?;
    match arguments[0].as_str() {
        "create-key" => serde_json::to_value(auth::create_key(db, id).await?),
        "list-keys" => serde_json::to_value(auth::list_keys(db, id).await?),
        "revoke-key" => serde_json::to_value(auth::revoke_key(db, id).await?),
        _ => unreachable!("arguments were validated"),
    }
    .map_err(AppError::internal)
}

#[test]
fn admin_command_shape() {
    for command in ["create-business", "create-key", "list-keys", "revoke-key"] {
        assert!(valid_admin_arguments(&[
            "admin".into(),
            command.into(),
            "Business or ID".into()
        ]));
    }
    for arguments in [
        vec![],
        vec!["admin"],
        vec!["create-business", "Name"],
        vec!["admin", "create-business"],
        vec!["admin", "unknown", "1"],
        vec!["admin", "list-keys", "1", "extra"],
    ] {
        assert!(!valid_admin_arguments(
            &arguments.into_iter().map(String::from).collect::<Vec<_>>()
        ));
    }
}
