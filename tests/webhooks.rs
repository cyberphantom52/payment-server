use payment_server::{AppState, auth, webhooks};

use axum::{Router, body::Bytes, extract::State, http::HeaderMap, middleware, routing::post};
use chrono::Utc;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

const SECRET: &str = "0707070707070707070707070707070707070707070707070707070707070707";
type Captures = Arc<Mutex<Vec<(String, String, String)>>>;

async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (url, task)
}

async fn fixture(failures: usize) -> (String, tokio::task::JoinHandle<()>, Captures) {
    async fn receive(
        State((captures, failures)): State<(Captures, usize)>,
        headers: HeaderMap,
        body: Bytes,
    ) -> StatusCode {
        let mut captures = captures.lock().unwrap();
        captures.push((
            String::from_utf8(body.to_vec()).unwrap(),
            headers["Webhook-Timestamp"].to_str().unwrap().to_owned(),
            headers["Webhook-Signature"].to_str().unwrap().to_owned(),
        ));
        if captures.len() <= failures {
            StatusCode::INTERNAL_SERVER_ERROR
        } else {
            StatusCode::NO_CONTENT
        }
    }
    let captures = Arc::new(Mutex::new(Vec::new()));
    let router = Router::new()
        .route("/hook", post(receive))
        .with_state((captures.clone(), failures));
    let (url, task) = serve(router).await;
    (format!("{url}/hook"), task, captures)
}

fn event(id: i64) -> String {
    json!({"id":id,"type":"invoice.created","created_at":"2026-10-01T00:00:00Z","data":{"invoice_id":1}}).to_string()
}

async fn seed(pool: &PgPool, url: Option<&str>, payload: &str) {
    sqlx::raw_sql("INSERT INTO businesses (name) VALUES ('One');
        INSERT INTO customers (business_id, name, email) VALUES (1, 'Alice', 'alice@example.com');
        INSERT INTO invoices (business_id, customer_id, due_date, total_cents) VALUES (1, 1, CURRENT_DATE, 100);")
        .execute(pool).await.unwrap();
    if let Some(url) = url {
        sqlx::query(
            "INSERT INTO webhook_endpoints (business_id, url, signing_secret) VALUES (1, $1, $2)",
        )
        .bind(url)
        .bind(SECRET)
        .execute(pool)
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO webhook_events (business_id, invoice_id, endpoint_id, event_type, payload, next_attempt_at)
        SELECT 1, 1, endpoint.id, 'invoice.created', $1, CASE WHEN endpoint.id IS NOT NULL THEN now() END
        FROM (SELECT 1) seed LEFT JOIN webhook_endpoints endpoint ON endpoint.business_id = 1")
        .bind(payload).execute(pool).await.unwrap();
}

#[test]
fn signature_and_url_boundaries() {
    // Independently calculated with Python's stdlib hmac/sha256.
    assert_eq!(
        webhooks::signature(SECRET, "1000", b"payload").unwrap(),
        "v1=f4c304488a9bf0dc726938876ab44d2913fdb2acbe7e1c65e0c83d687d2be778"
    );
    let body = event(1);
    let signed = webhooks::signature(SECRET, "1000", body.as_bytes()).unwrap();
    webhooks::verify_signature(SECRET, "1000", &signed, body.as_bytes(), 1000).unwrap();
    webhooks::verify_signature(SECRET, "1000", &signed, body.as_bytes(), 1300).unwrap();
    assert!(webhooks::verify_signature(SECRET, "1000", &signed, body.as_bytes(), 1301).is_err());
    assert!(webhooks::verify_signature(SECRET, "1000", &signed, body.as_bytes(), 699).is_err());
    assert!(webhooks::verify_signature(SECRET, "1000", &signed, b"altered", 1000).is_err());
    assert!(webhooks::signature("short", "1000", body.as_bytes()).is_err());
    assert!(webhooks::WebhookConfig::new(vec!["https://example.com/hook".into()], None).is_ok());
    for invalid in [
        "http://example.com/hook",
        "https://user:pass@example.com/hook",
        "https://example.com/hook#fragment",
        "file:///tmp/hook",
    ] {
        assert!(
            webhooks::WebhookConfig::new(vec![invalid.into()], None).is_err(),
            "{invalid}"
        );
    }
    assert!(
        webhooks::WebhookConfig::new(
            vec!["http://localhost:3001/demo/webhooks".into()],
            Some("http://localhost:3001/demo/webhooks".into())
        )
        .is_ok()
    );
    assert!(
        webhooks::WebhookConfig::new(vec![], Some("http://localhost:3001/demo/webhooks".into()))
            .is_err()
    );
}

async fn blocking_fixture() -> (
    String,
    tokio::task::JoinHandle<()>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    async fn receive(State(count): State<Arc<AtomicUsize>>) -> StatusCode {
        count.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(30)).await;
        StatusCode::NO_CONTENT
    }
    let count = Arc::new(AtomicUsize::new(0));
    let (url, server) = serve(
        Router::new()
            .route("/hook", post(receive))
            .with_state(count.clone()),
    )
    .await;
    (format!("{url}/hook"), server, count)
}

#[sqlx::test]
async fn cancelled_fifth_send_is_reserved_and_restart_cannot_send_sixth(pool: PgPool) {
    use std::sync::atomic::Ordering;
    let (url, server, count) = blocking_fixture().await;
    seed(&pool, Some(&url), &event(1)).await;
    sqlx::query("UPDATE webhook_events SET attempt_count = 4")
        .execute(&pool)
        .await
        .unwrap();
    let dispatch_pool = pool.clone();
    let dispatch = tokio::spawn(async move {
        webhooks::process_one(&dispatch_pool, &webhooks::delivery_client().unwrap()).await
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while count.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let reserved: (i64, bool, i64) = sqlx::query_as("SELECT attempt_count, last_attempt_at IS NOT NULL, round(extract(epoch FROM next_attempt_at - last_attempt_at))::bigint FROM webhook_events")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(reserved, (5, true, 5));
    dispatch.abort();
    assert!(dispatch.await.unwrap_err().is_cancelled());
    assert!(
        !webhooks::process_one(&pool, &webhooks::delivery_client().unwrap())
            .await
            .unwrap()
    );
    sqlx::query("UPDATE webhook_events SET next_attempt_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        webhooks::process_one(&pool, &webhooks::delivery_client().unwrap())
            .await
            .unwrap()
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let exhausted: (i64, bool, bool) = sqlx::query_as("SELECT attempt_count, exhausted_at IS NOT NULL, next_attempt_at IS NULL FROM webhook_events").fetch_one(&pool).await.unwrap();
    assert_eq!(exhausted, (5, true, true));
    server.abort();
}

#[sqlx::test]
async fn request_timeout_is_capped_by_remaining_event_lifetime(pool: PgPool) {
    use std::sync::atomic::Ordering;
    let (url, server, count) = blocking_fixture().await;
    seed(&pool, Some(&url), &event(1)).await;
    sqlx::query("UPDATE webhook_events SET created_at = now() - interval '15 minutes' + interval '250 milliseconds'").execute(&pool).await.unwrap();
    let started = std::time::Instant::now();
    assert!(
        webhooks::process_one(&pool, &webhooks::delivery_client().unwrap())
            .await
            .unwrap()
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let deadline_capped: bool = sqlx::query_scalar(
        "SELECT next_attempt_at = created_at + interval '15 minutes' FROM webhook_events",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(deadline_capped);
    // The DB budget expired during the blocked HTTP request; no retry is sent.
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(
        webhooks::process_one(&pool, &webhooks::delivery_client().unwrap())
            .await
            .unwrap()
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    server.abort();
}

#[sqlx::test]
async fn delivery_does_not_follow_redirects(pool: PgPool) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let target_count = Arc::new(AtomicUsize::new(0));
    async fn target(State(count): State<Arc<AtomicUsize>>) -> StatusCode {
        count.fetch_add(1, Ordering::SeqCst);
        StatusCode::NO_CONTENT
    }
    let router = Router::new()
        .route(
            "/hook",
            post(|| async { (StatusCode::TEMPORARY_REDIRECT, [("Location", "/target")]) }),
        )
        .route("/target", post(target))
        .with_state(target_count.clone());
    let (url, server) = serve(router).await;
    seed(&pool, Some(&format!("{url}/hook")), &event(1)).await;
    assert!(
        webhooks::process_one(&pool, &webhooks::delivery_client().unwrap())
            .await
            .unwrap()
    );
    assert_eq!(target_count.load(Ordering::SeqCst), 0);
    let delivered: bool = sqlx::query_scalar("SELECT delivered_at IS NOT NULL FROM webhook_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!delivered);
    server.abort();
}

#[sqlx::test]
async fn registration_is_scoped_allowlisted_and_secret_once(pool: PgPool) {
    let business = auth::create_business(&pool, "One").await.unwrap();
    let state = AppState { db: pool.clone() };
    let config =
        webhooks::WebhookConfig::new(vec!["https://example.com/hook".into()], None).unwrap();
    let router = webhooks::routes(config)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ))
        .with_state(state);
    let (url, server) = serve(router).await;
    let client = Client::new();
    for destination in [
        "http://example.com/hook",
        "https://other.example/hook",
        "https://example.com/other",
        "https://example.com/hook#x",
    ] {
        let response = client
            .post(format!("{url}/webhook-endpoints"))
            .bearer_auth(&business.api_key.key)
            .json(&json!({"url":destination}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(response.json::<Value>().await.unwrap()["error"].is_object());
    }
    let response = client
        .post(format!("{url}/webhook-endpoints"))
        .bearer_auth(&business.api_key.key)
        .json(&json!({"url":"https://EXAMPLE.com:443/hook"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let registered: Value = response.json().await.unwrap();
    assert_eq!(registered["url"], "https://example.com/hook");
    let secret = registered["signing_secret"].as_str().unwrap();
    assert_eq!(secret.len(), 64);
    assert!(secret.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let duplicate = client
        .post(format!("{url}/webhook-endpoints"))
        .bearer_auth(&business.api_key.key)
        .json(&json!({"url":"https://example.com/hook"}))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    assert!(
        duplicate
            .json::<Value>()
            .await
            .unwrap()
            .get("signing_secret")
            .is_none()
    );
    let unauthorized = client
        .post(format!("{url}/webhook-endpoints"))
        .json(&json!({"url":"https://example.com/hook"}))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    server.abort();
}

#[sqlx::test(migrations = "./provider_migrations")]
async fn receiver_verifies_and_persists_deduplicated_receipts(pool: PgPool) {
    let (url, server) = serve(webhooks::demo_router(pool.clone(), Some(SECRET)).unwrap()).await;
    let client = Client::new();
    let body = event(1);
    let timestamp = Utc::now().timestamp().to_string();
    let signature = webhooks::signature(SECRET, &timestamp, body.as_bytes()).unwrap();
    let wrong_method = client
        .get(format!("{url}/demo/webhooks"))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(wrong_method.json::<Value>().await.unwrap()["error"].is_object());
    let oversized = client
        .post(format!("{url}/demo/webhooks"))
        .body(vec![b'x'; 2 * 1024 * 1024 + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::BAD_REQUEST);
    assert!(oversized.json::<Value>().await.unwrap()["error"].is_object());
    let duplicate_headers = client
        .post(format!("{url}/demo/webhooks"))
        .header("Webhook-Timestamp", &timestamp)
        .header("Webhook-Signature", &signature)
        .header("Webhook-Signature", &signature)
        .body(body.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate_headers.status(), StatusCode::BAD_REQUEST);
    assert!(duplicate_headers.json::<Value>().await.unwrap()["error"].is_object());
    let invalid_id = event(0);
    let invalid = client
        .post(format!("{url}/demo/webhooks"))
        .header("Webhook-Timestamp", &timestamp)
        .header(
            "Webhook-Signature",
            webhooks::signature(SECRET, &timestamp, invalid_id.as_bytes()).unwrap(),
        )
        .body(invalid_id)
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert!(invalid.json::<Value>().await.unwrap()["error"].is_object());
    for _ in 0..2 {
        let response = client
            .post(format!("{url}/demo/webhooks"))
            .header("Webhook-Timestamp", &timestamp)
            .header("Webhook-Signature", &signature)
            .body(body.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    let stored: (i64, String) =
        sqlx::query_as("SELECT count(*) OVER (), payload FROM demo_webhook_receipts")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, (1, body.clone()));
    let altered = body.replace("invoice.created", "invoice.paid");
    let response = client
        .post(format!("{url}/demo/webhooks"))
        .header("Webhook-Timestamp", &timestamp)
        .header("Webhook-Signature", &signature)
        .body(altered.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let fresh = webhooks::signature(SECRET, &timestamp, altered.as_bytes()).unwrap();
    let response = client
        .post(format!("{url}/demo/webhooks"))
        .header("Webhook-Timestamp", &timestamp)
        .header("Webhook-Signature", fresh)
        .body(altered)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let stale = (Utc::now().timestamp() - 301).to_string();
    let response = client
        .post(format!("{url}/demo/webhooks"))
        .header("Webhook-Timestamp", &stale)
        .header(
            "Webhook-Signature",
            webhooks::signature(SECRET, &stale, body.as_bytes()).unwrap(),
        )
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    server.abort();
    let (url, restarted) = serve(webhooks::demo_router(pool.clone(), Some(SECRET)).unwrap()).await;
    let body = event(1);
    let response = client
        .post(format!("{url}/demo/webhooks"))
        .header("Webhook-Timestamp", &timestamp)
        .header("Webhook-Signature", signature)
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    restarted.abort();
}

#[sqlx::test]
async fn retry_preserves_payload_with_fresh_signed_headers(pool: PgPool) {
    let (url, server, captures) = fixture(1).await;
    let body = event(1);
    seed(&pool, Some(&url), &body).await;
    let client = webhooks::delivery_client().unwrap();
    assert!(webhooks::process_one(&pool, &client).await.unwrap());
    let persisted: (i64, i64, bool) = sqlx::query_as("SELECT attempt_count, round(extract(epoch FROM next_attempt_at - last_attempt_at))::bigint, delivered_at IS NULL FROM webhook_events")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(persisted, (1, 5, true));
    assert!(!webhooks::process_one(&pool, &client).await.unwrap());
    let first_timestamp = captures.lock().unwrap()[0].1.clone();
    while Utc::now().timestamp().to_string() == first_timestamp {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    sqlx::query("UPDATE webhook_events SET next_attempt_at = now() - interval '1 second'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(webhooks::process_one(&pool, &client).await.unwrap());
    let terminal: (i64, bool, bool) = sqlx::query_as("SELECT attempt_count, delivered_at IS NOT NULL, next_attempt_at IS NULL FROM webhook_events").fetch_one(&pool).await.unwrap();
    assert_eq!(terminal, (2, true, true));
    let captures = captures.lock().unwrap();
    assert_eq!(captures.len(), 2);
    assert_eq!(captures[0].0, body);
    assert_eq!(captures[1].0, body);
    assert_ne!(captures[0].1, captures[1].1);
    assert_ne!(captures[0].2, captures[1].2);
    for (payload, timestamp, signature) in captures.iter() {
        webhooks::verify_signature(
            SECRET,
            timestamp,
            signature,
            payload.as_bytes(),
            Utc::now().timestamp(),
        )
        .unwrap();
    }
    server.abort();
}

#[sqlx::test]
async fn retries_are_bounded_by_attempts_and_expiry(pool: PgPool) {
    let (url, server, captures) = fixture(usize::MAX).await;
    seed(&pool, Some(&url), &event(1)).await;
    let client = webhooks::delivery_client().unwrap();
    for (attempt, delay) in [(1_i64, 5_i64), (2, 10), (3, 20), (4, 40)] {
        assert!(webhooks::process_one(&pool, &client).await.unwrap());
        let result: (i64, i64) = sqlx::query_as("SELECT attempt_count, round(extract(epoch FROM next_attempt_at - last_attempt_at))::bigint FROM webhook_events")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(result, (attempt, delay));
        sqlx::query("UPDATE webhook_events SET next_attempt_at = now() - interval '1 second'")
            .execute(&pool)
            .await
            .unwrap();
    }
    // Simulate a crash after reserving the fifth send. Recovery cannot send a sixth.
    sqlx::query("UPDATE webhook_events SET attempt_count = 5, last_attempt_at = now(), next_attempt_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    assert!(webhooks::process_one(&pool, &client).await.unwrap());
    assert_eq!(captures.lock().unwrap().len(), 4);
    let exhausted: (bool, bool) = sqlx::query_as(
        "SELECT exhausted_at IS NOT NULL, next_attempt_at IS NULL FROM webhook_events",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(exhausted, (true, true));
    assert!(!webhooks::process_one(&pool, &client).await.unwrap());
    sqlx::query("INSERT INTO webhook_events (business_id, invoice_id, endpoint_id, event_type, payload, created_at, next_attempt_at)
        VALUES (1, 1, 1, 'invoice.paid', $1, now() - interval '16 minutes', now() + interval '1 hour')")
        .bind(event(2)).execute(&pool).await.unwrap();
    assert!(webhooks::process_one(&pool, &client).await.unwrap());
    assert_eq!(captures.lock().unwrap().len(), 4);
    let expired: (i64, bool) = sqlx::query_as(
        "SELECT attempt_count, exhausted_at IS NOT NULL FROM webhook_events WHERE id = 2",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(expired, (0, true));
    server.abort();
}

#[sqlx::test]
async fn missing_endpoint_is_never_backfilled(pool: PgPool) {
    let (url, server, captures) = fixture(0).await;
    seed(&pool, None, &event(1)).await;
    sqlx::query(
        "INSERT INTO webhook_endpoints (business_id, url, signing_secret) VALUES (1, $1, $2)",
    )
    .bind(url)
    .bind(SECRET)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        !webhooks::process_one(&pool, &webhooks::delivery_client().unwrap())
            .await
            .unwrap()
    );
    assert!(captures.lock().unwrap().is_empty());
    let result: (i64, bool, bool) = sqlx::query_as(
        "SELECT attempt_count, next_attempt_at IS NULL, endpoint_id IS NULL FROM webhook_events",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(result, (0, true, true));
    server.abort();
}
