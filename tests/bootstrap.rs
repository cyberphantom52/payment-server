use payment_server::{AppState, auth, bootstrap};

use axum::{Router, middleware, routing::get};
use reqwest::StatusCode;
use sqlx::PgPool;

const KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const SECRET: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const URL: &str = "http://payment-provider:3001/demo/webhooks";

#[sqlx::test]
async fn bootstrap_is_idempotent_and_never_revives_a_revoked_key(pool: PgPool) {
    bootstrap::seed(&pool, Some(KEY), Some(SECRET), Some(URL))
        .await
        .unwrap();
    bootstrap::seed(&pool, Some(KEY), Some(SECRET), Some(URL))
        .await
        .unwrap();
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM businesses), (SELECT count(*) FROM api_keys), (SELECT count(*) FROM webhook_endpoints)").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1));
    let stored: (String, String, String) = sqlx::query_as("SELECT key_hash, display_prefix, signing_secret FROM api_keys CROSS JOIN webhook_endpoints").fetch_one(&pool).await.unwrap();
    assert_eq!(
        stored,
        (auth::hash_key(KEY), KEY[..8].to_owned(), SECRET.to_owned())
    );
    auth::revoke_key(&pool, 1).await.unwrap();
    bootstrap::seed(&pool, Some(KEY), Some(SECRET), Some(URL))
        .await
        .unwrap();
    let state = AppState { db: pool.clone() };
    let router = Router::new()
        .route("/protected", get(|| async { StatusCode::NO_CONTENT }))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_api_key,
        ))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/protected", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(KEY)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.abort();
    assert!(
        bootstrap::seed(
            &pool,
            Some(KEY),
            Some(SECRET),
            Some("http://payment-provider:3001/changed")
        )
        .await
        .is_err()
    );
    let counts: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM businesses), (SELECT count(*) FROM api_keys), (SELECT count(*) FROM webhook_endpoints)").fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1));
}

#[sqlx::test]
async fn bootstrap_rejects_partial_or_invalid_configuration(pool: PgPool) {
    bootstrap::seed(&pool, None, None, None).await.unwrap();
    for (key, secret, url) in [
        (Some(KEY), None, None),
        (None, Some(SECRET), Some(URL)),
        (Some(KEY), Some(SECRET), None),
        (Some("short"), Some(SECRET), Some(URL)),
        (Some(KEY), Some("bad"), Some(URL)),
        (
            Some(KEY),
            Some(SECRET),
            Some("http://user:pass@example.com/hook"),
        ),
    ] {
        assert!(bootstrap::seed(&pool, key, secret, url).await.is_err());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM businesses")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
