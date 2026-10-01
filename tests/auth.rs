use axum::{
    Json, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    middleware,
    routing::{get, post},
};
use payment_server::{
    AppState,
    auth::{self, BusinessId},
    error::{ApiJson, ApiPath, ApiQuery},
    router,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::time::Duration;
use tower::ServiceExt;

async fn identity(BusinessId(id): BusinessId) -> Json<Value> {
    Json(json!({ "business_id": id }))
}

#[derive(Deserialize)]
struct Input {
    value: i64,
}

async fn input(ApiJson(value): ApiJson<Input>) -> Json<Value> {
    Json(json!({ "value": value.value }))
}

async fn parameter(ApiPath(value): ApiPath<i64>, ApiQuery(query): ApiQuery<Input>) -> Json<Value> {
    Json(json!({ "id": value, "value": query.value }))
}

async fn response(
    app: &Router,
    method: &str,
    uri: &str,
    authorization: Option<&str>,
    body: &str,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(value) = authorization {
        request = request.header("authorization", value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_owned())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[sqlx::test]
async fn key_lifecycle_and_request_authentication(pool: PgPool) {
    let first = auth::create_business(&pool, "  First  ").await.unwrap();
    let second = auth::create_business(&pool, "Second").await.unwrap();
    assert_eq!(first.business.name, "First");
    let raw_key = &first.api_key.key;
    assert_eq!(raw_key.len(), 64);
    assert!(raw_key.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let (hash, prefix): (String, String) =
        sqlx::query_as("SELECT key_hash, display_prefix FROM api_keys WHERE id = $1")
            .bind(first.api_key.metadata.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let expected_hash: String = Sha256::digest(raw_key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(hash, expected_hash);
    assert_ne!(hash, *raw_key);
    assert_eq!(prefix, raw_key[..8]);
    assert_ne!(second.api_key.key, *raw_key);
    let state = AppState { db: pool.clone() };
    let protected = Router::new()
        .route("/identity", get(identity))
        .route("/input", post(input))
        .route("/parameter/{id}", get(parameter))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ))
        .with_state(state.clone());
    let app = router(state).merge(protected);
    let bearer = format!("Bearer {raw_key}");
    let other_bearer = format!("Bearer {}", second.api_key.key);
    assert_eq!(
        response(
            &app,
            "GET",
            &format!("/identity?business_id={}", second.business.id),
            Some(&bearer),
            ""
        )
        .await,
        (StatusCode::OK, json!({ "business_id": first.business.id }))
    );
    assert_eq!(
        response(&app, "GET", "/identity", Some(&other_bearer), "").await,
        (StatusCode::OK, json!({ "business_id": second.business.id }))
    );
    for header in [
        None,
        Some("Basic nope"),
        Some("Bearer short"),
        Some("Bearer "),
        Some("Bearer 0000000000000000000000000000000000000000000000000000000000000000"),
    ] {
        let (status, body) = response(&app, "GET", "/identity", header, "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"]["code"], "unauthorized");
    }
    let request = Request::builder()
        .uri("/identity")
        .header("authorization", &bearer)
        .header("authorization", &other_bearer)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let replacement = auth::create_key(&pool, first.business.id).await.unwrap();
    let listed =
        serde_json::to_value(auth::list_keys(&pool, first.business.id).await.unwrap()).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 2);
    assert!(listed[0].get("key").is_none());
    assert!(listed[0].get("key_hash").is_none());
    assert!(listed[0]["created_at"].as_str().unwrap().ends_with('Z'));
    let revoked = auth::revoke_key(&pool, first.api_key.metadata.id)
        .await
        .unwrap();
    assert!(revoked.revoked_at.is_some());
    assert_eq!(
        auth::revoke_key(&pool, revoked.id)
            .await
            .unwrap()
            .revoked_at,
        revoked.revoked_at
    );
    assert_eq!(
        response(&app, "GET", "/identity", Some(&bearer), "")
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        response(
            &app,
            "GET",
            "/identity",
            Some(&format!("Bearer {}", replacement.key)),
            ""
        )
        .await
        .0,
        StatusCode::OK
    );
    for (method, path, body) in [
        ("POST", "/input", "{"),
        ("POST", "/input", "{\"value\":\"wrong\"}"),
        ("GET", "/parameter/nope?value=1", ""),
        ("GET", "/parameter/1?value=nope", ""),
    ] {
        let (status, value) = response(&app, method, path, Some(&other_bearer), body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], "invalid_input");
    }
    assert_eq!(
        response(&app, "GET", "/health", None, "").await,
        (StatusCode::OK, json!({"status":"ok"}))
    );
    assert_eq!(
        response(&app, "GET", "/absent", None, "").await.1["error"]["code"],
        "not_found"
    );
    assert_eq!(
        response(&app, "POST", "/health", None, "").await.1["error"]["code"],
        "method_not_allowed"
    );
    assert_eq!(
        auth::create_business(&pool, " \n ")
            .await
            .unwrap_err()
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        auth::create_key(&pool, i64::MAX).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        auth::list_keys(&pool, i64::MAX).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        auth::revoke_key(&pool, i64::MAX).await.unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    // Force initial key insertion to fail and verify the business transaction rolls back.
    sqlx::raw_sql("CREATE FUNCTION reject_key() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'test key failure'; END $$;
        CREATE TRIGGER reject_key BEFORE INSERT ON api_keys FOR EACH ROW EXECUTE FUNCTION reject_key();")
        .execute(&pool).await.unwrap();
    assert!(
        auth::create_business(&pool, "Must roll back")
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM businesses")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn readiness_failure_and_fallback_are_json() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(10))
        .connect_lazy("postgresql://localhost/postgres")
        .unwrap();
    pool.close().await;
    let app = router(AppState { db: pool });
    let (status, body) = response(&app, "GET", "/health", None, "").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        body,
        json!({"error":{"code":"unavailable","message":"Database is unavailable"}})
    );
    assert_eq!(
        response(&app, "GET", "/unknown", None, "").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        response(&app, "POST", "/health", None, "").await.0,
        StatusCode::METHOD_NOT_ALLOWED
    );
}
