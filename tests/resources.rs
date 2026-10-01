use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use payment_server::{AppState, auth, router};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

async fn request(
    app: &Router,
    key: &str,
    method: &str,
    path: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {key}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn invoice(customer_id: i64, items: Value) -> Value {
    json!({"customer_id":customer_id,"due_date":"2020-01-02","line_items":items})
}

fn items() -> Value {
    json!([
        {"description":"  First  ","quantity":2,"unit_amount_cents":100},
        {"description":"Free","quantity":1,"unit_amount_cents":0},
        {"description":"Last","quantity":1,"unit_amount_cents":50}
    ])
}

#[sqlx::test]
async fn resources_are_scoped_validated_paginated_and_atomic(pool: PgPool) {
    let first = auth::create_business(&pool, "One").await.unwrap();
    let second = auth::create_business(&pool, "Two").await.unwrap();
    let app = router(AppState { db: pool.clone() });
    let key = &first.api_key.key;
    for invalid in [
        json!({"name":" ","email":"alice@example.com"}),
        json!({"name":"Alice","email":"\n"}),
        json!({"name":"Alice","email":"a","business_id":second.business.id}),
    ] {
        let (status, body) = request(&app, key, "POST", "/customers", invalid).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid_input");
    }
    let (status, customer) = request(
        &app,
        key,
        "POST",
        "/customers",
        json!({"name":" Alice ","email":" alice@example.com "}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(customer["name"], "Alice");
    assert_eq!(customer["email"], "alice@example.com");
    assert!(customer["created_at"].as_str().unwrap().ends_with('Z'));
    let customer_id = customer["id"].as_i64().unwrap();
    let (_, other_customer) = request(
        &app,
        &second.api_key.key,
        "POST",
        "/customers",
        json!({"name":"Bob","email":"bob@example.com"}),
    )
    .await;
    let other_customer_id = other_customer["id"].as_i64().unwrap();
    assert_eq!(
        request(
            &app,
            key,
            "GET",
            &format!("/customers/{customer_id}"),
            Value::Null
        )
        .await
        .1,
        customer
    );
    for path in [
        format!("/customers/{other_customer_id}"),
        "/customers/9223372036854775807".into(),
    ] {
        assert_eq!(
            request(&app, key, "GET", &path, Value::Null).await.0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        request(
            &app,
            key,
            "POST",
            "/invoices",
            invoice(other_customer_id, items())
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, key, "POST", "/invoices", invoice(i64::MAX, items()))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, created) = request(
        &app,
        key,
        "POST",
        "/invoices",
        invoice(customer_id, items()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created["total_cents"], 250);
    assert_eq!(created["state"], "open");
    assert_eq!(created["paid_at"], Value::Null);
    assert_eq!(created["due_date"], "2020-01-02");
    assert_eq!(created["line_items"][0]["description"], "First");
    let invoice_id = created["id"].as_i64().unwrap();
    assert_eq!(
        request(
            &app,
            key,
            "GET",
            &format!("/invoices/{invoice_id}"),
            Value::Null
        )
        .await
        .1,
        created
    );
    assert_eq!(
        request(
            &app,
            &second.api_key.key,
            "GET",
            &format!("/invoices/{invoice_id}"),
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let event: (i64, String, Option<i64>, bool) = sqlx::query_as("SELECT id, payload, endpoint_id, next_attempt_at IS NULL FROM webhook_events WHERE invoice_id = $1")
        .bind(invoice_id).fetch_one(&pool).await.unwrap();
    let event_body: Value = serde_json::from_str(&event.1).unwrap();
    assert_eq!(event_body["id"], event.0);
    assert_eq!(event_body["type"], "invoice.created");
    assert_eq!(
        event_body["data"],
        json!({"invoice_id":invoice_id,"business_id":first.business.id,"customer_id":customer_id,"state":"open","total_cents":250})
    );
    assert!(event_body["created_at"].as_str().unwrap().ends_with('Z'));
    assert_eq!(event.2, None);
    assert!(event.3);
    let endpoint_id: i64 = sqlx::query_scalar("INSERT INTO webhook_endpoints (business_id, url, signing_secret) VALUES ($1, 'http://receiver/webhook', 'secret') RETURNING id")
        .bind(first.business.id).fetch_one(&pool).await.unwrap();
    let (_, second_invoice) = request(
        &app,
        key,
        "POST",
        "/invoices",
        invoice(customer_id, items()),
    )
    .await;
    let second_invoice_id = second_invoice["id"].as_i64().unwrap();
    let (captured, scheduled): (Option<i64>, bool) = sqlx::query_as(
        "SELECT endpoint_id, next_attempt_at IS NOT NULL FROM webhook_events WHERE invoice_id = $1",
    )
    .bind(second_invoice_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(captured, Some(endpoint_id));
    assert!(scheduled);
    assert_eq!(
        sqlx::query_scalar::<_, bool>(
            "SELECT next_attempt_at IS NULL FROM webhook_events WHERE invoice_id = $1"
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        true
    );
    sqlx::query("UPDATE invoices SET state = 'paid', paid_at = now() WHERE id = $1")
        .bind(second_invoice_id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, list) = request(&app, key, "GET", "/invoices?limit=1", Value::Null).await;
    assert_eq!(list["data"].as_array().unwrap().len(), 1);
    assert_eq!(list["data"][0]["id"], invoice_id);
    assert!(list["data"][0].get("line_items").is_none());
    assert_eq!(list["next_after_id"], invoice_id);
    let (_, next) = request(
        &app,
        key,
        "GET",
        &format!("/invoices?limit=1&after_id={invoice_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(next["data"][0]["id"], second_invoice_id);
    assert_eq!(next["next_after_id"], Value::Null);
    for (state, expected_id) in [("open", invoice_id), ("paid", second_invoice_id)] {
        let (_, list) = request(
            &app,
            key,
            "GET",
            &format!("/invoices?state={state}"),
            Value::Null,
        )
        .await;
        assert_eq!(list["data"].as_array().unwrap().len(), 1);
        assert_eq!(list["data"][0]["id"], expected_id);
    }
    assert_eq!(
        request(&app, &second.api_key.key, "GET", "/invoices", Value::Null)
            .await
            .1["data"],
        json!([])
    );
    for query in [
        "limit=0",
        "limit=-1",
        "limit=101",
        "after_id=-1",
        "limit=abc",
    ] {
        for path in ["/customers", "/invoices"] {
            assert_eq!(
                request(&app, key, "GET", &format!("{path}?{query}"), Value::Null)
                    .await
                    .0,
                StatusCode::BAD_REQUEST
            );
        }
    }
    assert_eq!(
        request(&app, key, "GET", "/invoices?state=void", Value::Null)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    for path in [
        "/customers?limit=100&after_id=0",
        "/invoices?limit=100&after_id=0",
    ] {
        assert_eq!(
            request(&app, key, "GET", path, Value::Null).await.0,
            StatusCode::OK
        );
    }
    sqlx::query("INSERT INTO customers (business_id, name, email) SELECT $1, 'Extra', 'extra@example.com' FROM generate_series(1,21)")
        .bind(first.business.id).execute(&pool).await.unwrap();
    let (_, default_page) = request(&app, key, "GET", "/customers", Value::Null).await;
    assert_eq!(default_page["data"].as_array().unwrap().len(), 20);
    let after = default_page["next_after_id"].as_i64().unwrap();
    let (_, final_page) = request(
        &app,
        key,
        "GET",
        &format!("/customers?after_id={after}"),
        Value::Null,
    )
    .await;
    assert_eq!(final_page["data"].as_array().unwrap().len(), 2);
    assert_eq!(final_page["next_after_id"], Value::Null);
    assert!(
        final_page["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|customer| customer["business_id"] == first.business.id)
    );
    let mut invalid_date = invoice(customer_id, items());
    invalid_date["due_date"] = json!("2020-02-30");
    let mut client_total = invoice(customer_id, items());
    client_total["total_cents"] = json!(1);
    let invalid_items = [
        json!([]),
        json!([{"description":" ","quantity":1,"unit_amount_cents":1}]),
        json!([{"description":"Item","quantity":0,"unit_amount_cents":1}]),
        json!([{"description":"Item","quantity":1,"unit_amount_cents":-1}]),
        json!([{"description":"Item","quantity":1,"unit_amount_cents":0}]),
        json!([{"description":"Item","quantity":2,"unit_amount_cents":i64::MAX}]),
        json!([{"description":"Max","quantity":1,"unit_amount_cents":i64::MAX},{"description":"Overflow","quantity":1,"unit_amount_cents":1}]),
        json!([{"description":"Item","quantity":1.5,"unit_amount_cents":1}]),
        json!([{"description":"Item","quantity":1,"unit_amount_cents":1,"total":1}]),
    ];
    for invalid in invalid_items
        .into_iter()
        .map(|items| invoice(customer_id, items))
        .chain([invalid_date, client_total])
    {
        let (status, body) = request(&app, key, "POST", "/invoices", invalid).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"]["code"], "invalid_input");
    }
    let (status, maximum) = request(
        &app,
        key,
        "POST",
        "/invoices",
        invoice(
            customer_id,
            json!([{"description":"Maximum","quantity":1,"unit_amount_cents":i64::MAX}]),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(maximum["total_cents"], i64::MAX);
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM invoices), (SELECT count(*) FROM invoice_line_items), (SELECT count(*) FROM webhook_events)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (3, 7, 3));
    sqlx::raw_sql("CREATE FUNCTION reject_event() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'forced private event failure'; END $$;
        CREATE TRIGGER reject_event BEFORE INSERT ON webhook_events FOR EACH ROW EXECUTE FUNCTION reject_event();")
        .execute(&pool).await.unwrap();
    let (status, body) = request(
        &app,
        key,
        "POST",
        "/invoices",
        invoice(customer_id, items()),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        body,
        json!({"error":{"code":"internal_error","message":"An internal error occurred"}})
    );
    let final_counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM invoices), (SELECT count(*) FROM invoice_line_items), (SELECT count(*) FROM webhook_events)")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(counts, final_counts);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT payload FROM webhook_events WHERE id = $1")
            .bind(event.0)
            .fetch_one(&pool)
            .await
            .unwrap(),
        event.1
    );
}
