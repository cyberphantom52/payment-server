use payment_server::provider;

use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;

async fn serve(pool: PgPool) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, provider::router(pool)).await.unwrap();
    });
    (url, task)
}

fn payment(token: &str) -> Value {
    json!({"invoice_id": 9, "amount_cents": 100, "card_token": token})
}

#[sqlx::test(migrations = "./provider_migrations")]
async fn duplicate_operations_conflicts_and_failures(pool: PgPool) {
    let (url, server) = serve(pool.clone()).await;
    let client = Client::new();
    let first = client
        .post(format!("{url}/payments"))
        .header("Idempotency-Key", "1")
        .json(&payment("tok_success"))
        .send();
    let second = client
        .post(format!("{url}/payments"))
        .header("Idempotency-Key", "1")
        .json(&payment("tok_success"))
        .send();
    let (first, second) = tokio::join!(first, second);
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(second.status(), StatusCode::OK);
    let first: Value = first.json().await.unwrap();
    assert_eq!(first, second.json::<Value>().await.unwrap());
    assert_eq!(first["status"], "succeeded");
    assert_eq!(first["attempt_id"], 1);
    assert!(first["psp_ref"].as_str().unwrap().len() == 36);

    for changed in [
        json!({"invoice_id": 10, "amount_cents": 100, "card_token": "tok_success"}),
        json!({"invoice_id": 9, "amount_cents": 101, "card_token": "tok_success"}),
        payment("tok_card_declined"),
        payment("unknown"),
    ] {
        let response = client
            .post(format!("{url}/payments"))
            .header("Idempotency-Key", "1")
            .json(&changed)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(response.json::<Value>().await.unwrap()["error"].is_object());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM provider_operations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    for (id, token, failure_code) in [
        (2, "tok_card_declined", "card_declined"),
        (3, "tok_insufficient_funds", "insufficient_funds"),
    ] {
        let response = client
            .post(format!("{url}/payments"))
            .header("Idempotency-Key", id.to_string())
            .json(&payment(token))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: Value = response.json().await.unwrap();
        assert_eq!(result["status"], "failed");
        assert_eq!(result["code"], failure_code);
        assert!(result.get("psp_ref").is_none());
    }
    server.abort();
}

#[sqlx::test(migrations = "./provider_migrations")]
async fn network_error_has_a_durable_success(pool: PgPool) {
    let (url, server) = serve(pool.clone()).await;
    let client = Client::new();
    for _ in 0..2 {
        let response = client
            .post(format!("{url}/payments"))
            .header("Idempotency-Key", "1")
            .json(&payment("tok_network_error"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(response.json::<Value>().await.unwrap()["error"].is_object());
    }
    let result: Value = client
        .get(format!("{url}/payments/1"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let stored: (i64, String) =
        sqlx::query_as("SELECT count(*) OVER (), psp_ref::text FROM provider_operations")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored.0, 1);
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["attempt_id"], 1);
    assert_eq!(result["psp_ref"], stored.1);
    server.abort();
}

#[sqlx::test(migrations = "./provider_migrations")]
async fn cancelled_request_survives_restart_and_reuses_deadline(pool: PgPool) {
    let (url, server) = serve(pool.clone()).await;
    let client = Client::new();
    let cancelled = client
        .post(format!("{url}/payments"))
        .header("Idempotency-Key", "1")
        .json(&payment("tok_timeout"))
        .timeout(Duration::from_millis(250))
        .send()
        .await
        .unwrap_err();
    assert!(cancelled.is_timeout());
    let persisted: (String, String) = sqlx::query_as(
        "SELECT psp_ref::text, ready_at::text FROM provider_operations WHERE attempt_id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    server.abort();
    let (url, restarted) = serve(pool.clone()).await;
    let pending: Value = client
        .get(format!("{url}/payments/1"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pending, json!({"attempt_id": 1, "status": "pending"}));
    let after_restart: (String, String) = sqlx::query_as(
        "SELECT psp_ref::text, ready_at::text FROM provider_operations WHERE attempt_id = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted, after_restart);

    // Advance the test operation's persisted deadline to avoid a 30-second test.
    sqlx::query("UPDATE provider_operations SET ready_at = now() + interval '100 milliseconds'")
        .execute(&pool)
        .await
        .unwrap();
    let response = client
        .post(format!("{url}/payments"))
        .header("Idempotency-Key", "1")
        .json(&payment("tok_timeout"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["psp_ref"], persisted.0);
    restarted.abort();
}

#[sqlx::test(migrations = "./provider_migrations")]
async fn validation_and_health_return_json(pool: PgPool) {
    let (url, server) = serve(pool.clone()).await;
    let client = Client::new();
    for (key, body) in [
        (None, payment("tok_success")),
        (Some("0"), payment("tok_success")),
        (Some("-1"), payment("tok_success")),
        (Some("1.5"), payment("tok_success")),
        (Some("+1"), payment("tok_success")),
        (Some("9223372036854775808"), payment("tok_success")),
        (Some("1"), payment("unknown")),
        (
            Some("1"),
            json!({"invoice_id": 0, "amount_cents": 100, "card_token": "tok_success"}),
        ),
        (
            Some("1"),
            json!({"invoice_id": 9, "amount_cents": 0, "card_token": "tok_success"}),
        ),
        (
            Some("1"),
            json!({"invoice_id": 9, "amount_cents": 1.5, "card_token": "tok_success"}),
        ),
        (Some("1"), json!({"invoice_id": 9, "amount_cents": 100})),
    ] {
        let mut request = client.post(format!("{url}/payments")).json(&body);
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        assert!(response.json::<Value>().await.unwrap()["error"].is_object());
    }
    let duplicated = client
        .post(format!("{url}/payments"))
        .header("Idempotency-Key", "1")
        .header("Idempotency-Key", "2")
        .json(&payment("tok_success"))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicated.status(), StatusCode::BAD_REQUEST);
    assert!(duplicated.json::<Value>().await.unwrap()["error"].is_object());
    for (path, status) in [
        ("/payments/1", StatusCode::NOT_FOUND),
        ("/payments/no-id", StatusCode::BAD_REQUEST),
        ("/payments/0", StatusCode::BAD_REQUEST),
    ] {
        let response = client.get(format!("{url}{path}")).send().await.unwrap();
        assert_eq!(response.status(), status);
        assert!(response.json::<Value>().await.unwrap()["error"].is_object());
    }
    let response = client.get(format!("{url}/health")).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"status": "ok"})
    );
    pool.close().await;
    let response = client.get(format!("{url}/health")).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.json::<Value>().await.unwrap()["error"].is_object());
    server.abort();
}
