use crate::error::{ApiJson, ApiPath, AppError};
use axum::{Json, Router, extract::State, http::HeaderMap, routing::get};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::time::Duration;

pub fn router(db: PgPool) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/payments", axum::routing::post(pay))
        .route("/payments/{attempt_id}", get(lookup))
        .fallback(|| async { AppError::not_found() })
        .method_not_allowed_fallback(|| async { AppError::method_not_allowed() })
        .with_state(db)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PaymentRequest {
    invoice_id: i64,
    amount_cents: i64,
    card_token: String,
}

#[derive(sqlx::FromRow)]
struct Operation {
    attempt_id: i64,
    invoice_id: i64,
    amount_cents: i64,
    mock_card_token: String,
    psp_ref: String,
    planned_outcome: String,
    ready_in_ms: i64,
}

impl Operation {
    fn response(&self) -> Value {
        if self.ready_in_ms > 0 {
            json!({"attempt_id": self.attempt_id, "status": "pending"})
        } else if self.planned_outcome == "succeeded" {
            json!({"attempt_id": self.attempt_id, "status": "succeeded", "psp_ref": self.psp_ref})
        } else {
            json!({"attempt_id": self.attempt_id, "status": "failed", "code": self.planned_outcome})
        }
    }
}

async fn operation(db: &PgPool, attempt_id: i64) -> Result<Operation, AppError> {
    sqlx::query_as(
        "SELECT attempt_id, invoice_id, amount_cents, mock_card_token, psp_ref::text,
                planned_outcome, ceil(GREATEST(extract(epoch FROM ready_at - now()) * 1000, 0))::bigint AS ready_in_ms
         FROM provider_operations WHERE attempt_id = $1",
    )
    .bind(attempt_id)
    .fetch_optional(db)
    .await?
    .ok_or_else(AppError::not_found)
}

async fn pay(
    State(db): State<PgPool>,
    headers: HeaderMap,
    ApiJson(request): ApiJson<PaymentRequest>,
) -> Result<Json<Value>, AppError> {
    let mut keys = headers.get_all("Idempotency-Key").iter();
    let attempt_id = keys
        .next()
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|id| *id > 0)
        .ok_or_else(|| AppError::invalid_input("Idempotency-Key must be a positive attempt ID"))?;
    if keys.next().is_some() {
        return Err(AppError::invalid_input("Send exactly one Idempotency-Key"));
    }
    if request.invoice_id <= 0 || request.amount_cents <= 0 {
        return Err(AppError::invalid_input(
            "invoice_id and amount_cents must be positive integers",
        ));
    }
    let (outcome, delay_ms) = match request.card_token.as_str() {
        "tok_success" => ("succeeded", 100_i64),
        "tok_insufficient_funds" => ("insufficient_funds", 100),
        "tok_card_declined" => ("card_declined", 100),
        "tok_timeout" => ("succeeded", 30000),
        "tok_network_error" => ("succeeded", 0),
        _ => {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM provider_operations WHERE attempt_id = $1)",
            )
            .bind(attempt_id)
            .fetch_one(&db)
            .await?;
            return Err(if exists {
                AppError::conflict("Idempotency-Key was already used with different payment input")
            } else {
                AppError::invalid_input("Unknown card_token")
            });
        }
    };

    // Autocommit makes the operation durable before any response delay. A second
    // statement sees the winning row even when a concurrent insert had to finish.
    sqlx::query(
        "INSERT INTO provider_operations
            (attempt_id, invoice_id, amount_cents, mock_card_token, psp_ref, planned_outcome, ready_at)
         VALUES ($1, $2, $3, $4, gen_random_uuid(), $5, now() + $6::bigint * interval '1 millisecond')
         ON CONFLICT (attempt_id) DO NOTHING",
    )
    .bind(attempt_id)
    .bind(request.invoice_id)
    .bind(request.amount_cents)
    .bind(&request.card_token)
    .bind(outcome)
    .bind(delay_ms)
    .execute(&db)
    .await?;

    let mut stored = operation(&db, attempt_id).await?;
    if stored.invoice_id != request.invoice_id
        || stored.amount_cents != request.amount_cents
        || stored.mock_card_token != request.card_token
    {
        return Err(AppError::conflict(
            "Idempotency-Key was already used with different payment input",
        ));
    }
    if stored.mock_card_token == "tok_network_error" {
        return Err(AppError::internal("Simulated provider network error"));
    }
    while stored.ready_in_ms > 0 {
        tokio::time::sleep(Duration::from_millis(stored.ready_in_ms as u64)).await;
        stored = operation(&db, attempt_id).await?;
    }
    Ok(Json(stored.response()))
}

async fn lookup(
    State(db): State<PgPool>,
    ApiPath(attempt_id): ApiPath<i64>,
) -> Result<Json<Value>, AppError> {
    if attempt_id <= 0 {
        return Err(AppError::invalid_input("attempt_id must be positive"));
    }
    Ok(Json(operation(&db, attempt_id).await?.response()))
}

async fn health(State(db): State<PgPool>) -> Result<Json<Value>, AppError> {
    sqlx::query("SELECT 1")
        .execute(&db)
        .await
        .map_err(|error| {
            eprintln!("provider database readiness error: {error}");
            AppError::unavailable("Database is unavailable")
        })?;
    Ok(Json(json!({"status": "ok"})))
}
