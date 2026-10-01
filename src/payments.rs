use crate::{
    AppState,
    auth::BusinessId,
    entities::payment_attempt::{Attempt, Replay, Work},
    error::{ApiJson, ApiPath, ApiQuery, AppError},
    events::insert_event,
    models::{
        data::{
            page::{Page, page},
            provider_payment::ProviderResult,
        },
        enums::failure_code::FailureCode,
        forms::{list::ListQuery, payment::PaymentInput},
    },
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use reqwest::Client;
use serde_json::json;
use sqlx::{PgConnection, PgPool};
use std::time::Duration;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/invoices/{id}/pay", post(pay))
        .route("/payment-attempts/{id}", get(get_attempt))
        .route("/invoices/{id}/payment-attempts", get(list_attempts))
}

impl Replay {
    fn response(self, invoice_id: i64, token: &str) -> Result<Response, AppError> {
        if self.invoice_id != invoice_id || self.mock_card_token != token {
            return Err(AppError::conflict(
                "Idempotency-Key was already used with different payment input",
            ));
        }
        Ok((
            StatusCode::ACCEPTED,
            [
                (header::CONTENT_TYPE, "application/json".to_owned()),
                (header::LOCATION, format!("/payment-attempts/{}", self.id)),
            ],
            self.accepted_response_body,
        )
            .into_response())
    }
}

async fn replay(
    connection: &mut PgConnection,
    business_id: i64,
    key: &str,
) -> Result<Option<Replay>, AppError> {
    Ok(sqlx::query_as("SELECT id, invoice_id, mock_card_token, accepted_response_body FROM payment_attempts WHERE business_id = $1 AND idempotency_key = $2")
        .bind(business_id).bind(key).fetch_optional(connection).await?)
}

async fn pay(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiPath(invoice_id): ApiPath<i64>,
    headers: HeaderMap,
    ApiJson(input): ApiJson<PaymentInput>,
) -> Result<Response, AppError> {
    let mut keys = headers.get_all("Idempotency-Key").iter();
    let key = keys
        .next()
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty() && value.len() <= 255)
        .ok_or_else(|| {
            AppError::invalid_input(
                "Idempotency-Key must be nonempty and at most 255 ASCII characters",
            )
        })?;
    if keys.next().is_some() {
        return Err(AppError::invalid_input("Send exactly one Idempotency-Key"));
    }
    let mut connection = state.db.acquire().await?;
    if let Some(stored) = replay(&mut connection, business_id, key).await? {
        return stored.response(invoice_id, &input.card_token);
    }
    drop(connection);
    if !matches!(
        input.card_token.as_str(),
        "tok_success"
            | "tok_insufficient_funds"
            | "tok_card_declined"
            | "tok_timeout"
            | "tok_network_error"
    ) {
        return Err(AppError::invalid_input("Unknown card_token"));
    }
    let mut transaction = state.db.begin().await?;
    let invoice_state: String = sqlx::query_scalar(
        "SELECT state FROM invoices WHERE business_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(business_id)
    .bind(invoice_id)
    .fetch_one(&mut *transaction)
    .await?;
    // Another request for this invoice may have committed while we waited for its lock.
    if let Some(stored) = replay(&mut transaction, business_id, key).await? {
        return stored.response(invoice_id, &input.card_token);
    }
    if invoice_state != "open" {
        return Err(AppError::conflict("Invoice is already paid"));
    }
    let pending: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM payment_attempts WHERE invoice_id = $1 AND state = 'pending')")
        .bind(invoice_id).fetch_one(&mut *transaction).await?;
    if pending {
        return Err(AppError::conflict("Invoice already has a pending payment"));
    }
    let inserted = sqlx::query_scalar::<_, i64>("INSERT INTO payment_attempts
        (business_id, invoice_id, idempotency_key, mock_card_token, accepted_response_body, next_check_at)
        VALUES ($1, $2, $3, $4, '', now()) RETURNING id")
        .bind(business_id).bind(invoice_id).bind(key).bind(&input.card_token).fetch_one(&mut *transaction).await;
    let id = match inserted {
        Ok(id) => id,
        Err(error)
            if error
                .as_database_error()
                .and_then(|error| error.constraint())
                == Some("payment_attempts_business_id_idempotency_key_key") =>
        {
            // A different invoice using the same business/key can win without taking our invoice lock.
            transaction.rollback().await?;
            let mut connection = state.db.acquire().await?;
            return replay(&mut connection, business_id, key)
                .await?
                .ok_or_else(|| AppError::internal("Winning idempotency row disappeared"))?
                .response(invoice_id, &input.card_token);
        }
        Err(error) => return Err(error.into()),
    };
    let body = serde_json::to_string(
        &json!({"payment_attempt_id":id,"invoice_id":invoice_id,"status":"pending"}),
    )
    .map_err(AppError::internal)?;
    sqlx::query("UPDATE payment_attempts SET accepted_response_body = $1 WHERE id = $2")
        .bind(&body)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Replay {
        id,
        invoice_id,
        mock_card_token: input.card_token.clone(),
        accepted_response_body: body,
    }
    .response(invoice_id, &input.card_token)
}

async fn get_attempt(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<Attempt>, AppError> {
    Ok(Json(sqlx::query_as("SELECT id, business_id, invoice_id, state, psp_ref::text, failure_code, created_at, updated_at
        FROM payment_attempts WHERE business_id = $1 AND id = $2")
        .bind(business_id).bind(id).fetch_one(&state.db).await?))
}

async fn list_attempts(
    State(state): State<AppState>,
    BusinessId(business_id): BusinessId,
    ApiPath(invoice_id): ApiPath<i64>,
    ApiQuery(query): ApiQuery<ListQuery>,
) -> Result<Json<Page<Attempt>>, AppError> {
    let (limit, after_id) = query.pagination()?;
    if query.state.is_some() {
        return Err(AppError::invalid_input(
            "Payment attempt lists do not support a state filter",
        ));
    }
    sqlx::query_scalar::<_, i64>("SELECT id FROM invoices WHERE business_id = $1 AND id = $2")
        .bind(business_id)
        .bind(invoice_id)
        .fetch_one(&state.db)
        .await?;
    let attempts = sqlx::query_as("SELECT id, business_id, invoice_id, state, psp_ref::text, failure_code, created_at, updated_at
        FROM payment_attempts WHERE business_id = $1 AND invoice_id = $2 AND id > $3 ORDER BY id LIMIT $4")
        .bind(business_id).bind(invoice_id).bind(after_id).bind(limit + 1).fetch_all(&state.db).await?;
    Ok(Json(page(attempts, limit, |attempt: &Attempt| attempt.id)))
}

pub struct Worker {
    db: PgPool,
    client: Client,
    psp_url: String,
}

impl Worker {
    pub fn new(db: PgPool, psp_url: &str) -> Result<Self, AppError> {
        let url = reqwest::Url::parse(psp_url).map_err(AppError::internal)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(AppError::internal(
                "PSP_URL must be an absolute HTTP(S) URL",
            ));
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(AppError::internal)?;
        Ok(Self {
            db,
            client,
            psp_url: psp_url.trim_end_matches('/').to_owned(),
        })
    }

    /// Reads due work without holding a database transaction across the provider call.
    pub async fn process_one(&self) -> Result<bool, AppError> {
        // ponytail: one sequential worker; provider idempotency and guarded finalization
        // also make duplicate workers safe, add leases only if duplicate polls matter.
        let Some(work) = sqlx::query_as::<_, Work>(
            "SELECT attempt.id, attempt.business_id, attempt.invoice_id,
            attempt.mock_card_token, invoice.total_cents FROM payment_attempts attempt
            JOIN invoices invoice ON invoice.id = attempt.invoice_id
            WHERE attempt.state = 'pending' AND attempt.next_check_at <= now()
            ORDER BY attempt.next_check_at, attempt.id LIMIT 1",
        )
        .fetch_optional(&self.db)
        .await?
        else {
            return Ok(false);
        };
        match self.provider_result(&work).await {
            Some(result) if !matches!(result, ProviderResult::Pending { .. }) => {
                self.finalize(&work, result).await?
            }
            _ => {
                sqlx::query("UPDATE payment_attempts SET next_check_at = now() + interval '5 seconds', updated_at = now() WHERE id = $1 AND state = 'pending'")
                    .bind(work.id).execute(&self.db).await?;
            }
        }
        Ok(true)
    }

    async fn provider_result(&self, work: &Work) -> Option<ProviderResult> {
        let result = async {
            let lookup = self.client.get(format!("{}/payments/{}", self.psp_url, work.id)).send().await?;
            let response = if lookup.status() == reqwest::StatusCode::NOT_FOUND {
                self.client.post(format!("{}/payments", self.psp_url)).header("Idempotency-Key", work.id.to_string())
                    .json(&json!({"invoice_id":work.invoice_id,"amount_cents":work.total_cents,"card_token":work.mock_card_token}))
                    .send().await?
            } else { lookup };
            let response = response.error_for_status()?;
            if response.status() != reqwest::StatusCode::OK {
                eprintln!("provider returned unexpected HTTP status {} for {}", response.status(), work.id);
                return Ok(None);
            }
            response.json::<ProviderResult>().await.map(Some)
        }.await;
        match result {
            Ok(Some(result)) if result.attempt_id() == work.id => Some(result),
            Ok(Some(_)) => {
                eprintln!("provider response had the wrong attempt ID for {}", work.id);
                None
            }
            Ok(None) => None,
            Err(error) => {
                eprintln!("provider result unknown for {}: {error}", work.id);
                None
            }
        }
    }

    async fn finalize(&self, work: &Work, result: ProviderResult) -> Result<(), AppError> {
        let mut transaction = self.db.begin().await?;
        let invoice_state: String = sqlx::query_scalar(
            "SELECT state FROM invoices WHERE id = $1 AND business_id = $2 FOR UPDATE",
        )
        .bind(work.invoice_id)
        .bind(work.business_id)
        .fetch_one(&mut *transaction)
        .await?;
        let attempt_state: String =
            sqlx::query_scalar("SELECT state FROM payment_attempts WHERE id = $1 FOR UPDATE")
                .bind(work.id)
                .fetch_one(&mut *transaction)
                .await?;
        if attempt_state != "pending" {
            return Ok(());
        }
        if invoice_state != "open" {
            return Err(AppError::internal(
                "Pending attempt belongs to a paid invoice",
            ));
        }
        let (status, psp_ref, failure_code, event_type) = match result {
            ProviderResult::Succeeded { psp_ref, .. } => {
                sqlx::query("UPDATE invoices SET state = 'paid', paid_at = now() WHERE id = $1")
                    .bind(work.invoice_id)
                    .execute(&mut *transaction)
                    .await?;
                ("succeeded", Some(psp_ref.to_string()), None, "invoice.paid")
            }
            ProviderResult::Failed { code, .. } => {
                let code = match code {
                    FailureCode::CardDeclined => "card_declined",
                    FailureCode::InsufficientFunds => "insufficient_funds",
                };
                ("failed", None, Some(code), "invoice.payment_failed")
            }
            ProviderResult::Pending { .. } => return Ok(()),
        };
        sqlx::query("UPDATE payment_attempts SET state = $1, psp_ref = $2::text::uuid, failure_code = $3, next_check_at = NULL, updated_at = now() WHERE id = $4")
            .bind(status).bind(&psp_ref).bind(failure_code).bind(work.id).execute(&mut *transaction).await?;
        let invoice_state = if status == "succeeded" {
            "paid"
        } else {
            "open"
        };
        insert_event(&mut transaction, work.business_id, work.invoice_id, event_type,
            &json!({"business_id":work.business_id,"invoice_id":work.invoice_id,"payment_attempt_id":work.id,
                "state":invoice_state,"total_cents":work.total_cents,"status":status,"psp_ref":psp_ref,"failure_code":failure_code})).await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn run(&self) {
        loop {
            match self.process_one().await {
                Ok(true) => {}
                Ok(false) => tokio::time::sleep(Duration::from_millis(500)).await,
                Err(error) => {
                    eprintln!("payment worker error: {}", error.message);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }
}
