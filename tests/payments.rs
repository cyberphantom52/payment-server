use axum::{
    Router,
    extract::Request,
    http::Method,
    middleware::{self, Next},
};
use payment_server::{AppState, auth, payments::Worker, provider, router};
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use sqlx::{
    PgPool, Postgres,
    testing::{TestArgs, TestSupport},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

struct Server {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(app: Router) -> Server {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server { url, task }
}

async fn provider_db(path: &'static str) -> (PgPool, String) {
    // Use SQLx's same isolated-database support; provider tables never share the app database.
    let context = Postgres::test_context(&TestArgs::new(path)).await.unwrap();
    let pool = context
        .pool_opts
        .connect_with(context.connect_opts)
        .await
        .unwrap();
    sqlx::migrate!("./provider_migrations")
        .run(&pool)
        .await
        .unwrap();
    (pool, context.db_name)
}

async fn clean_provider(pool: PgPool, name: &str) {
    pool.close().await;
    Postgres::cleanup_test(name).await.unwrap();
}

async fn counted_provider(pool: PgPool) -> (Server, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let all = Arc::new(AtomicUsize::new(0));
    let posts = Arc::new(AtomicUsize::new(0));
    let (all_count, post_count) = (all.clone(), posts.clone());
    let app =
        provider::router(pool).layer(middleware::from_fn(move |request: Request, next: Next| {
            let (all_count, post_count) = (all_count.clone(), post_count.clone());
            async move {
                all_count.fetch_add(1, Ordering::SeqCst);
                if request.method() == Method::POST {
                    post_count.fetch_add(1, Ordering::SeqCst);
                }
                next.run(request).await
            }
        }));
    (serve(app).await, all, posts)
}

async fn seed(pool: &PgPool) -> (String, String, i64, i64) {
    let first = auth::create_business(pool, "One").await.unwrap();
    let second = auth::create_business(pool, "Two").await.unwrap();
    let customer: i64 = sqlx::query_scalar("INSERT INTO customers (business_id,name,email) VALUES ($1,'Alice','alice@example.com') RETURNING id")
        .bind(first.business.id).fetch_one(pool).await.unwrap();
    let mut ids = Vec::new();
    for _ in 0..2 {
        ids.push(sqlx::query_scalar::<_, i64>("INSERT INTO invoices (business_id,customer_id,due_date,total_cents) VALUES ($1,$2,CURRENT_DATE,100) RETURNING id")
            .bind(first.business.id).bind(customer).fetch_one(pool).await.unwrap());
    }
    (first.api_key.key, second.api_key.key, ids[0], ids[1])
}

async fn pay(
    client: &Client,
    url: &str,
    api_key: &str,
    invoice_id: i64,
    key: &str,
    token: &str,
) -> (StatusCode, String, String) {
    let response = client
        .post(format!("{url}/invoices/{invoice_id}/pay"))
        .bearer_auth(api_key)
        .header("Idempotency-Key", key)
        .json(&json!({"card_token":token}))
        .send()
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get("location")
        .map(|value| value.to_str().unwrap().to_owned())
        .unwrap_or_default();
    (status, response.text().await.unwrap(), location)
}

async fn status(pool: &PgPool, id: i64) -> String {
    sqlx::query_scalar("SELECT state FROM payment_attempts WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn make_due(pool: &PgPool) {
    sqlx::query("UPDATE payment_attempts SET next_check_at = now() WHERE state = 'pending'")
        .execute(pool)
        .await
        .unwrap();
}

async fn count(pool: &PgPool, statement: &'static str) -> i64 {
    sqlx::query_scalar(statement).fetch_one(pool).await.unwrap()
}

#[sqlx::test]
async fn concurrent_admission_replay_and_scoping(pool: PgPool) {
    let (provider_pool, provider_name) = provider_db("payments::concurrent_provider").await;
    let (psp, calls, posts) = counted_provider(provider_pool.clone()).await;
    let app = serve(router(AppState { db: pool.clone() })).await;
    let (key, other_key, invoice, another_invoice) = seed(&pool).await;
    let client = Client::new();
    let mut requests = tokio::task::JoinSet::new();
    for index in 0..20 {
        let (client, url, key) = (client.clone(), app.url.clone(), key.clone());
        requests.spawn(async move {
            pay(
                &client,
                &url,
                &key,
                invoice,
                &format!("competing-{index}"),
                "tok_success",
            )
            .await
        });
    }
    let mut accepted = Vec::new();
    while let Some(result) = requests.join_next().await {
        let result = result.unwrap();
        if result.0 == StatusCode::ACCEPTED {
            accepted.push(result);
        } else {
            assert_eq!(result.0, StatusCode::CONFLICT);
        }
    }
    assert_eq!(accepted.len(), 1);
    let attempt = serde_json::from_str::<Value>(&accepted[0].1).unwrap()["payment_attempt_id"]
        .as_i64()
        .unwrap();
    assert_eq!(accepted[0].2, format!("/payment-attempts/{attempt}"));
    assert_eq!(
        count(&pool, "SELECT count(*) FROM payment_attempts").await,
        1
    );
    let winning_key: String =
        sqlx::query_scalar("SELECT idempotency_key FROM payment_attempts WHERE id = $1")
            .bind(attempt)
            .fetch_one(&pool)
            .await
            .unwrap();
    let worker = Worker::new(pool.clone(), &psp.url).unwrap();
    assert!(worker.process_one().await.unwrap());
    assert_eq!(status(&pool, attempt).await, "succeeded");
    assert_eq!(
        count(&provider_pool, "SELECT count(*) FROM provider_operations").await,
        1
    );
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    let requests_before = calls.load(Ordering::SeqCst);
    for _ in 0..3 {
        assert_eq!(
            pay(
                &client,
                &app.url,
                &key,
                invoice,
                &winning_key,
                "tok_success"
            )
            .await,
            accepted[0]
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), requests_before);
    assert_eq!(
        pay(
            &client,
            &app.url,
            &key,
            invoice,
            &winning_key,
            "tok_card_declined"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        pay(
            &client,
            &app.url,
            &key,
            another_invoice,
            &winning_key,
            "tok_success"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        pay(
            &client,
            &app.url,
            &key,
            invoice,
            "paid-new-key",
            "tok_success"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        pay(
            &client,
            &app.url,
            &other_key,
            invoice,
            "cross-business",
            "tok_success"
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for path in [
        format!("/payment-attempts/{attempt}"),
        format!("/invoices/{invoice}/payment-attempts"),
    ] {
        assert_eq!(
            client
                .get(format!("{}{path}", app.url))
                .bearer_auth(&other_key)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let current: Value = client
        .get(format!("{}/payment-attempts/{attempt}", app.url))
        .bearer_auth(&key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(current["status"], "succeeded");
    assert!(current.get("card_token").is_none());
    assert!(current.get("mock_card_token").is_none());
    assert!(current["created_at"].as_str().unwrap().ends_with('Z'));
    let final_state: (String, bool) =
        sqlx::query_as("SELECT state, paid_at IS NOT NULL FROM invoices WHERE id = $1")
            .bind(invoice)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(final_state, ("paid".into(), true));
    let event: Value = serde_json::from_str(
        &sqlx::query_scalar::<_, String>(
            "SELECT payload FROM webhook_events WHERE invoice_id = $1",
        )
        .bind(invoice)
        .fetch_one(&pool)
        .await
        .unwrap(),
    )
    .unwrap();
    assert_eq!(event["type"], "invoice.paid");
    assert_eq!(event["data"]["payment_attempt_id"], attempt);
    assert_eq!(event["data"]["status"], "succeeded");
    assert_eq!(event["data"]["state"], "paid");
    assert!(!worker.process_one().await.unwrap());
    assert_eq!(count(&pool, "SELECT count(*) FROM webhook_events").await, 1);
    // Concurrent matching admission responses must be byte-for-byte identical.
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..20 {
        let (client, url, key) = (client.clone(), app.url.clone(), key.clone());
        requests.spawn(async move {
            pay(
                &client,
                &url,
                &key,
                another_invoice,
                "same-key",
                "tok_success",
            )
            .await
        });
    }
    let mut matches = Vec::new();
    while let Some(result) = requests.join_next().await {
        matches.push(result.unwrap());
    }
    assert_eq!(matches[0].0, StatusCode::ACCEPTED);
    assert!(matches.iter().all(|response| response == &matches[0]));
    assert_eq!(
        pay(
            &client,
            &app.url,
            &key,
            another_invoice,
            "pending-new-key",
            "tok_success"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert!(worker.process_one().await.unwrap());
    assert_eq!(
        count(&provider_pool, "SELECT count(*) FROM provider_operations").await,
        2
    );
    drop(psp);
    clean_provider(provider_pool, &provider_name).await;
}

#[sqlx::test]
async fn declines_unknown_results_and_atomic_finalization(pool: PgPool) {
    let (provider_pool, provider_name) = provider_db("payments::decline_provider").await;
    let psp = serve(provider::router(provider_pool.clone())).await;
    let app = serve(router(AppState { db: pool.clone() })).await;
    let (key, _, invoice, another_invoice) = seed(&pool).await;
    let client = Client::new();
    let declined = pay(
        &client,
        &app.url,
        &key,
        invoice,
        "decline",
        "tok_card_declined",
    )
    .await;
    let attempt = serde_json::from_str::<Value>(&declined.1).unwrap()["payment_attempt_id"]
        .as_i64()
        .unwrap();
    let worker = Worker::new(pool.clone(), &psp.url).unwrap();
    worker.process_one().await.unwrap();
    assert_eq!(status(&pool, attempt).await, "failed");
    let (state, code): (String, String) = sqlx::query_as("SELECT invoice.state, attempt.failure_code FROM invoices invoice JOIN payment_attempts attempt ON attempt.invoice_id = invoice.id WHERE attempt.id = $1")
        .bind(attempt).fetch_one(&pool).await.unwrap();
    assert_eq!((state.as_str(), code.as_str()), ("open", "card_declined"));
    assert_eq!(
        pay(
            &client,
            &app.url,
            &key,
            invoice,
            "decline",
            "tok_card_declined"
        )
        .await,
        declined
    );
    let retry = pay(&client, &app.url, &key, invoice, "retry", "tok_success").await;
    assert_eq!(retry.0, StatusCode::ACCEPTED);
    let retry_id = serde_json::from_str::<Value>(&retry.1).unwrap()["payment_attempt_id"]
        .as_i64()
        .unwrap();
    // Finalization failure rolls back both settlement and its event, preserving recovery.
    sqlx::raw_sql("CREATE FUNCTION reject_settlement_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'forced settlement failure'; END $$;
        CREATE TRIGGER reject_settlement_event BEFORE INSERT ON webhook_events FOR EACH ROW EXECUTE FUNCTION reject_settlement_event();")
        .execute(&pool).await.unwrap();
    assert!(worker.process_one().await.is_err());
    assert_eq!(status(&pool, retry_id).await, "pending");
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM invoices WHERE id = $1")
            .bind(invoice)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "open"
    );
    assert_eq!(count(&pool, "SELECT count(*) FROM webhook_events").await, 1);
    sqlx::query("DROP TRIGGER reject_settlement_event ON webhook_events")
        .execute(&pool)
        .await
        .unwrap();
    worker.process_one().await.unwrap();
    assert_eq!(status(&pool, retry_id).await, "succeeded");
    assert_eq!(count(&pool, "SELECT count(*) FROM webhook_events").await, 2);
    // Retry list pagination includes failed and succeeded attempts without exposing tokens.
    let page: Value = client
        .get(format!(
            "{}/invoices/{invoice}/payment-attempts?limit=1",
            app.url
        ))
        .bearer_auth(&key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["data"][0]["id"], attempt);
    assert_eq!(page["next_after_id"], attempt);
    let page: Value = client
        .get(format!(
            "{}/invoices/{invoice}/payment-attempts?limit=1&after_id={attempt}",
            app.url
        ))
        .bearer_auth(&key)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(page["data"][0]["id"], retry_id);
    assert_eq!(page["next_after_id"], Value::Null);
    let malformed = pay(
        &client,
        &app.url,
        &key,
        another_invoice,
        "malformed",
        "tok_success",
    )
    .await;
    let malformed_id = serde_json::from_str::<Value>(&malformed.1).unwrap()["payment_attempt_id"]
        .as_i64()
        .unwrap();
    for body in [
        json!({"attempt_id":malformed_id+1,"status":"succeeded","psp_ref":"00000000-0000-4000-8000-000000000001"}),
        json!({"attempt_id":malformed_id,"status":"succeeded","psp_ref":"invalid"}),
        json!({"attempt_id":malformed_id,"status":"failed","code":"unknown"}),
        json!({"attempt_id":malformed_id,"status":"unknown"}),
    ] {
        let mock = serve(Router::new().route(
            "/payments/{id}",
            axum::routing::get(move || {
                let body = body.clone();
                async { axum::Json(body) }
            }),
        ))
        .await;
        Worker::new(pool.clone(), &mock.url)
            .unwrap()
            .process_one()
            .await
            .unwrap();
        assert_eq!(status(&pool, malformed_id).await, "pending");
        let scheduled: bool =
            sqlx::query_scalar("SELECT next_check_at > now() FROM payment_attempts WHERE id = $1")
                .bind(malformed_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(scheduled);
        make_due(&pool).await;
    }
    // A terminal-looking redirect is not a confirmed provider result.
    let redirect = serve(Router::new().route("/payments/{id}", axum::routing::get(move || async move {
        (axum::http::StatusCode::FOUND, [("location", "http://127.0.0.1:1")], axum::Json(json!({"attempt_id":malformed_id,"status":"succeeded","psp_ref":"00000000-0000-4000-8000-000000000001"})))
    }))).await;
    Worker::new(pool.clone(), &redirect.url)
        .unwrap()
        .process_one()
        .await
        .unwrap();
    assert_eq!(status(&pool, malformed_id).await, "pending");
    drop(psp);
    clean_provider(provider_pool, &provider_name).await;
}

#[sqlx::test]
async fn network_loss_and_crash_recover_via_provider_lookup(pool: PgPool) {
    let (provider_pool, provider_name) = provider_db("payments::recovery_provider").await;
    let (psp, calls, posts) = counted_provider(provider_pool.clone()).await;
    let app = serve(router(AppState { db: pool.clone() })).await;
    let (key, _, invoice, another_invoice) = seed(&pool).await;
    let client = Client::new();
    let accepted = pay(
        &client,
        &app.url,
        &key,
        invoice,
        "network",
        "tok_network_error",
    )
    .await;
    let id = serde_json::from_str::<Value>(&accepted.1).unwrap()["payment_attempt_id"]
        .as_i64()
        .unwrap();
    let worker = Worker::new(pool.clone(), &psp.url).unwrap();
    worker.process_one().await.unwrap();
    assert_eq!(status(&pool, id).await, "pending");
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(
        count(&provider_pool, "SELECT count(*) FROM provider_operations").await,
        1
    );
    make_due(&pool).await;
    worker.process_one().await.unwrap();
    assert_eq!(status(&pool, id).await, "succeeded");
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let accepted = pay(
        &client,
        &app.url,
        &key,
        another_invoice,
        "crash",
        "tok_success",
    )
    .await;
    let crash_id = serde_json::from_str::<Value>(&accepted.1).unwrap()["payment_attempt_id"]
        .as_i64()
        .unwrap();
    // Provider commits, then the application dies before persisting settlement.
    let response = client
        .post(format!("{}/payments", psp.url))
        .header("Idempotency-Key", crash_id.to_string())
        .json(&json!({"invoice_id":another_invoice,"amount_cents":100,"card_token":"tok_success"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let committed: Value = response.json().await.unwrap();
    assert_eq!(status(&pool, crash_id).await, "pending");
    drop(worker);
    drop(psp);
    let (restarted_provider, _, restarted_posts) = counted_provider(provider_pool.clone()).await;
    Worker::new(pool.clone(), &restarted_provider.url)
        .unwrap()
        .process_one()
        .await
        .unwrap();
    assert_eq!(status(&pool, crash_id).await, "succeeded");
    assert_eq!(restarted_posts.load(Ordering::SeqCst), 0);
    assert_eq!(
        count(&provider_pool, "SELECT count(*) FROM provider_operations").await,
        2
    );
    let stored: String =
        sqlx::query_scalar("SELECT psp_ref::text FROM payment_attempts WHERE id = $1")
            .bind(crash_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, committed["psp_ref"]);
    drop(restarted_provider);
    clean_provider(provider_pool, &provider_name).await;
}

#[sqlx::test]
async fn real_thirty_second_timeout_is_pending_then_eventually_paid(pool: PgPool) {
    let (provider_pool, provider_name) = provider_db("payments::real_timeout_provider").await;
    let psp = serve(provider::router(provider_pool.clone())).await;
    let app = serve(router(AppState { db: pool.clone() })).await;
    let (key, _, invoice, _) = seed(&pool).await;
    let client = Client::new();
    let started = Instant::now();
    let accepted = pay(&client, &app.url, &key, invoice, "timeout", "tok_timeout").await;
    assert_eq!(accepted.0, StatusCode::ACCEPTED);
    assert!(started.elapsed() < Duration::from_secs(1));
    let id = serde_json::from_str::<Value>(&accepted.1).unwrap()["payment_attempt_id"]
        .as_i64()
        .unwrap();
    let worker = Worker::new(pool.clone(), &psp.url).unwrap();
    let attempt_started = Instant::now();
    worker.process_one().await.unwrap();
    assert!(attempt_started.elapsed() >= Duration::from_millis(1900));
    assert!(attempt_started.elapsed() < Duration::from_secs(4));
    assert_eq!(status(&pool, id).await, "pending");
    assert_eq!(
        pay(
            &client,
            &app.url,
            &key,
            invoice,
            "timeout-second",
            "tok_success"
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let mut processed = 0;
    while status(&pool, id).await == "pending" {
        assert!(
            started.elapsed() < Duration::from_secs(40),
            "timed-out provider operation did not settle"
        );
        if worker.process_one().await.unwrap() {
            processed += 1;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(started.elapsed() >= Duration::from_secs(30));
    assert!(processed >= 1);
    assert_eq!(status(&pool, id).await, "succeeded");
    assert_eq!(
        count(&provider_pool, "SELECT count(*) FROM provider_operations").await,
        1
    );
    assert_eq!(
        pay(&client, &app.url, &key, invoice, "timeout", "tok_timeout").await,
        accepted
    );
    drop(psp);
    clean_provider(provider_pool, &provider_name).await;
}

#[sqlx::test]
async fn input_validation_and_cross_invoice_key_race(pool: PgPool) {
    let app = serve(router(AppState { db: pool.clone() })).await;
    let (key, _, invoice, another_invoice) = seed(&pool).await;
    let client = Client::new();
    for (idempotency, body) in [
        (None, json!({"card_token":"tok_success"})),
        (Some(" ".into()), json!({"card_token":"tok_success"})),
        (Some("x".repeat(256)), json!({"card_token":"tok_success"})),
        (Some("valid".into()), json!({"card_token":"unknown"})),
        (
            Some("valid".into()),
            json!({"card_token":"tok_success","extra":true}),
        ),
    ] {
        let mut request = client
            .post(format!("{}/invoices/{invoice}/pay", app.url))
            .bearer_auth(&key)
            .json(&body);
        if let Some(value) = idempotency {
            request = request.header("Idempotency-Key", value);
        }
        let response = request.send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["code"],
            "invalid_input"
        );
    }
    let duplicate = client
        .post(format!("{}/invoices/{invoice}/pay", app.url))
        .bearer_auth(&key)
        .header("Idempotency-Key", "first")
        .header("Idempotency-Key", "second")
        .json(&json!({"card_token":"tok_success"}))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::BAD_REQUEST);
    let (first, second) = tokio::join!(
        pay(
            &client,
            &app.url,
            &key,
            invoice,
            "cross-invoice-race",
            "tok_success"
        ),
        pay(
            &client,
            &app.url,
            &key,
            another_invoice,
            "cross-invoice-race",
            "tok_success"
        )
    );
    assert!(
        (first.0 == StatusCode::ACCEPTED && second.0 == StatusCode::CONFLICT)
            || (second.0 == StatusCode::ACCEPTED && first.0 == StatusCode::CONFLICT)
    );
    assert_eq!(
        count(&pool, "SELECT count(*) FROM payment_attempts").await,
        1
    );
}
