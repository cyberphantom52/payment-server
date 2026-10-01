use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgConnection;

/// Call inside the transaction changing the invoice. The payload is immutable.
pub async fn insert_event(
    connection: &mut PgConnection,
    business_id: i64,
    invoice_id: i64,
    event_type: &str,
    data: &Value,
) -> Result<(), AppError> {
    let (id, created_at): (i64, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO webhook_events (business_id, invoice_id, endpoint_id, event_type, payload, next_attempt_at)
         SELECT $1, $2, endpoint.id, $3, '', CASE WHEN endpoint.id IS NOT NULL THEN now() END
         FROM (SELECT 1) seed LEFT JOIN webhook_endpoints endpoint ON endpoint.business_id = $1
         RETURNING id, created_at",
    ).bind(business_id).bind(invoice_id).bind(event_type).fetch_one(&mut *connection).await?;
    let payload = serde_json::to_string(
        &json!({"id":id, "type":event_type, "created_at":created_at, "data":data}),
    )
    .map_err(AppError::internal)?;
    sqlx::query("UPDATE webhook_events SET payload = $1 WHERE id = $2")
        .bind(payload)
        .bind(id)
        .execute(connection)
        .await?;
    Ok(())
}
