use crate::{
    AppState,
    auth::{BusinessId, hex},
    entities::webhook_delivery::Delivery,
    error::{ApiJson, AppError},
    models::{data::webhook_event::Event, forms::webhook_endpoint::EndpointRequest},
};
use axum::{
    Extension, Json, Router,
    body::Bytes,
    extract::{State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use chrono::Utc;
use hmac::{Hmac, KeyInit, Mac};
use reqwest::{Client, Url};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::PgPool;
use std::time::Duration;

#[derive(Clone, Default)]
pub struct WebhookConfig {
    allowed_urls: Vec<String>,
    demo_url: Option<String>,
}

impl WebhookConfig {
    pub fn new(allowed_urls: Vec<String>, demo_url: Option<String>) -> Result<Self, AppError> {
        let demo_url = demo_url.map(|url| canonical_url(&url)).transpose()?;
        let allowed_urls = allowed_urls
            .iter()
            .map(|url| canonical_url(url))
            .collect::<Result<Vec<_>, _>>()?;
        let config = Self {
            allowed_urls,
            demo_url,
        };
        for url in &config.allowed_urls {
            config.validate(url)?;
        }
        if let Some(url) = &config.demo_url {
            config.validate(url)?;
        }
        Ok(config)
    }

    pub(crate) fn validate(&self, raw: &str) -> Result<String, AppError> {
        let canonical = canonical_url(raw)?;
        let url = Url::parse(&canonical).map_err(AppError::internal)?;
        if !self.allowed_urls.contains(&canonical)
            || (url.scheme() != "https" && self.demo_url.as_deref() != Some(&canonical))
        {
            return Err(AppError::invalid_input(
                "Webhook URL must be operator-allowed HTTPS or the exact configured demo URL",
            ));
        }
        Ok(canonical)
    }
}

fn canonical_url(raw: &str) -> Result<String, AppError> {
    let url = Url::parse(raw).map_err(|_| AppError::invalid_input("Invalid webhook URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(AppError::invalid_input(
            "Webhook URL must use HTTP(S), with no credentials or fragment",
        ));
    }
    Ok(url.to_string())
}

pub fn routes(config: WebhookConfig) -> Router<AppState> {
    Router::new()
        .route("/webhook-endpoints", post(register))
        .layer(Extension(config))
}

async fn register(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    Extension(config): Extension<WebhookConfig>,
    ApiJson(request): ApiJson<EndpointRequest>,
) -> Result<(StatusCode, Json<Value>), AppError> {
    let url = config.validate(&request.url)?;
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(AppError::internal)?;
    let secret = hex(&bytes);
    let id: i64 = sqlx::query_scalar("INSERT INTO webhook_endpoints (business_id, url, signing_secret) VALUES ($1, $2, $3) RETURNING id")
        .bind(business_id).bind(&url).bind(&secret).fetch_one(&state.db).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({"id":id, "url":url, "signing_secret":secret})),
    ))
}

fn key_bytes(secret: &str) -> Result<[u8; 32], AppError> {
    if secret.len() != 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::invalid_input(
            "Signing secret must be exactly 32 hex-encoded bytes",
        ));
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&secret[index * 2..index * 2 + 2], 16)
            .map_err(|_| AppError::invalid_input("Invalid signing secret"))?;
    }
    Ok(bytes)
}

fn mac(secret: &[u8; 32], timestamp: &str, body: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts a 32-byte key");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    mac
}

pub fn signature(secret: &str, timestamp: &str, body: &[u8]) -> Result<String, AppError> {
    Ok(format!(
        "v1={}",
        hex(&mac(&key_bytes(secret)?, timestamp, body)
            .finalize()
            .into_bytes())
    ))
}

pub fn verify_signature(
    secret: &str,
    timestamp: &str,
    signature: &str,
    body: &[u8],
    now: i64,
) -> Result<(), AppError> {
    if timestamp.is_empty() || !timestamp.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(AppError::invalid_input("Invalid webhook timestamp"));
    }
    let signed_at = timestamp
        .parse::<i64>()
        .map_err(|_| AppError::invalid_input("Invalid webhook timestamp"))?;
    if now.abs_diff(signed_at) > 300 {
        return Err(AppError::invalid_input(
            "Webhook timestamp is outside the five-minute window",
        ));
    }
    let digest = key_bytes(
        signature
            .strip_prefix("v1=")
            .ok_or_else(|| AppError::invalid_input("Invalid webhook signature"))?,
    )?;
    mac(&key_bytes(secret)?, timestamp, body)
        .verify_slice(&digest)
        .map_err(|_| AppError::invalid_input("Invalid webhook signature"))
}

#[derive(Clone)]
struct DemoState {
    db: PgPool,
    secret: Option<String>,
}

pub fn demo_router(db: PgPool, secret: Option<&str>) -> Result<Router, AppError> {
    if let Some(secret) = secret {
        key_bytes(secret)?;
    }
    Ok(Router::new()
        .route("/demo/webhooks", post(receive_demo))
        .method_not_allowed_fallback(|| async { AppError::method_not_allowed() })
        .with_state(DemoState {
            db,
            secret: secret.map(str::to_owned),
        }))
}

fn single_header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, AppError> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::invalid_input(format!("Missing or invalid {name}")))?;
    if values.next().is_some() {
        return Err(AppError::invalid_input(format!("Duplicate {name}")));
    }
    Ok(value)
}

async fn receive_demo(
    State(state): State<DemoState>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<StatusCode, AppError> {
    let body = body.map_err(|_| AppError::invalid_input("Invalid webhook body"))?;
    let secret = state
        .secret
        .as_deref()
        .ok_or_else(|| AppError::unavailable("Demo webhook receiver is not configured"))?;
    verify_signature(
        secret,
        single_header(&headers, "Webhook-Timestamp")?,
        single_header(&headers, "Webhook-Signature")?,
        &body,
        Utc::now().timestamp(),
    )?;
    let event: Event = serde_json::from_slice(&body)
        .map_err(|_| AppError::invalid_input("Invalid webhook event"))?;
    if event.id <= 0
        || !matches!(
            event.event_type.as_str(),
            "invoice.created" | "invoice.paid" | "invoice.payment_failed"
        )
        || !event.data.is_object()
        || event.created_at.timestamp() < 0
    {
        return Err(AppError::invalid_input("Invalid webhook event"));
    }
    let body =
        std::str::from_utf8(&body).map_err(|_| AppError::invalid_input("Invalid webhook event"))?;
    let inserted = sqlx::query("INSERT INTO demo_webhook_receipts (event_id, payload) VALUES ($1, $2) ON CONFLICT (event_id) DO NOTHING")
        .bind(event.id).bind(body).execute(&state.db).await?.rows_affected() == 1;
    let original: String =
        sqlx::query_scalar("SELECT payload FROM demo_webhook_receipts WHERE event_id = $1")
            .bind(event.id)
            .fetch_one(&state.db)
            .await?;
    if original != body {
        return Err(AppError::conflict(
            "Event ID was already received with a different payload",
        ));
    }
    if inserted {
        println!(
            "Verified demo webhook received: event_id={} type={}",
            event.id, event.event_type
        );
    }
    Ok(StatusCode::NO_CONTENT)
}

pub fn delivery_client() -> Result<Client, AppError> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        // Each transport dispatch must consume its own persisted attempt.
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(AppError::internal)
}

pub async fn run(db: PgPool, client: Client) -> Result<(), AppError> {
    loop {
        match process_one(&db, &client).await {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => eprintln!("webhook worker error: {}", error.message),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn retry_delay(attempt: i64) -> i64 {
    5 * (1_i64 << (attempt - 1))
}

/// Run from one sequential worker. Errors must return to its backoff loop.
pub async fn process_one(db: &PgPool, client: &Client) -> Result<bool, AppError> {
    // ponytail: one worker instance, add database leases before running replicas.
    let mut tx = db.begin().await?;
    let event = sqlx::query_as::<_, Delivery>(
        "SELECT event.id, event.payload, endpoint.url, endpoint.signing_secret, event.attempt_count,
            floor(extract(epoch FROM event.created_at + interval '15 minutes' - now()) * 1000)::bigint AS remaining_ms
         FROM webhook_events event JOIN webhook_endpoints endpoint ON endpoint.id = event.endpoint_id
         WHERE event.delivered_at IS NULL AND event.exhausted_at IS NULL AND event.next_attempt_at IS NOT NULL
           AND (event.next_attempt_at <= now() OR event.created_at + interval '15 minutes' <= now())
         ORDER BY event.next_attempt_at, event.id FOR UPDATE OF event SKIP LOCKED LIMIT 1")
        .fetch_optional(&mut *tx).await?;
    let Some(event) = event else {
        tx.commit().await?;
        return Ok(false);
    };
    if event.attempt_count >= 5 || event.remaining_ms <= 0 {
        sqlx::query(
            "UPDATE webhook_events SET exhausted_at = now(), next_attempt_at = NULL WHERE id = $1",
        )
        .bind(event.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(true);
    }
    let attempt = event.attempt_count + 1;
    let delay = if attempt < 5 { retry_delay(attempt) } else { 0 };
    sqlx::query("UPDATE webhook_events SET attempt_count = $2, last_attempt_at = now(),
        next_attempt_at = LEAST(now() + $3::bigint * interval '1 second', created_at + interval '15 minutes') WHERE id = $1")
        .bind(event.id).bind(attempt).bind(5 + delay).execute(&mut *tx).await?;
    tx.commit().await?;

    // Recheck the DB deadline after the durable reservation and before dispatch.
    let remaining_ms: i64 = sqlx::query_scalar("SELECT floor(extract(epoch FROM created_at + interval '15 minutes' - now()) * 1000)::bigint FROM webhook_events WHERE id = $1")
        .bind(event.id).fetch_one(db).await?;
    if remaining_ms <= 0 {
        sqlx::query(
            "UPDATE webhook_events SET exhausted_at = now(), next_attempt_at = NULL WHERE id = $1",
        )
        .bind(event.id)
        .execute(db)
        .await?;
        return Ok(true);
    }
    let timestamp = Utc::now().timestamp().to_string();
    let signature = signature(&event.signing_secret, &timestamp, event.payload.as_bytes())?;
    let delivered = client
        .post(&event.url)
        .header("Content-Type", "application/json")
        .header("Webhook-Timestamp", timestamp)
        .header("Webhook-Signature", signature)
        .body(event.payload)
        .timeout(Duration::from_millis(remaining_ms.min(5000) as u64))
        .send()
        .await
        .is_ok_and(|response| response.status().is_success());
    if delivered {
        sqlx::query(
            "UPDATE webhook_events SET delivered_at = now(), next_attempt_at = NULL WHERE id = $1",
        )
        .bind(event.id)
        .execute(db)
        .await?;
    } else if attempt >= 5 {
        sqlx::query(
            "UPDATE webhook_events SET exhausted_at = now(), next_attempt_at = NULL WHERE id = $1",
        )
        .bind(event.id)
        .execute(db)
        .await?;
    } else {
        sqlx::query("UPDATE webhook_events SET next_attempt_at = LEAST(now() + $2::bigint * interval '1 second', created_at + interval '15 minutes') WHERE id = $1")
            .bind(event.id).bind(delay).execute(db).await?;
    }
    Ok(true)
}
