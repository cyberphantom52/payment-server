use crate::{auth::hash_key, error::AppError, webhooks::WebhookConfig};
use sqlx::PgPool;

pub async fn seed(
    db: &PgPool,
    api_key: Option<&str>,
    secret: Option<&str>,
    demo_url: Option<&str>,
) -> Result<(), AppError> {
    let (api_key, secret, demo_url) = match (api_key, secret, demo_url) {
        (None, None, None) => return Ok(()),
        (Some(key), Some(secret), Some(url)) => (key, secret, url),
        _ => {
            return Err(AppError::invalid_input(
                "DEMO_API_KEY, DEMO_WEBHOOK_SECRET, and DEMO_WEBHOOK_URL must be configured together",
            ));
        }
    };
    if [api_key, secret]
        .iter()
        .any(|value| value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(AppError::invalid_input(
            "Demo API key and webhook secret must each be exactly 32 hex-encoded bytes",
        ));
    }
    let config = WebhookConfig::new(vec![demo_url.to_owned()], Some(demo_url.to_owned()))?;
    let url = config.validate(demo_url)?;
    let mut tx = db.begin().await?;
    // Serialize startup only; ordinary API and worker traffic do not take this lock.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('payment-server-demo-bootstrap'))")
        .execute(&mut *tx)
        .await?;
    let existing: Option<i64> =
        sqlx::query_scalar("SELECT business_id FROM api_keys WHERE key_hash = $1")
            .bind(hash_key(api_key))
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(business_id) = existing {
        let endpoint: Option<(String, String)> = sqlx::query_as(
            "SELECT url, signing_secret FROM webhook_endpoints WHERE business_id = $1",
        )
        .bind(business_id)
        .fetch_optional(&mut *tx)
        .await?;
        if endpoint != Some((url, secret.to_owned())) {
            return Err(AppError::conflict(
                "Existing demo webhook configuration differs from DEMO_WEBHOOK_URL or DEMO_WEBHOOK_SECRET",
            ));
        }
        // A revoked key deliberately stays revoked across restarts.
    } else {
        let business_id: i64 = sqlx::query_scalar(
            "INSERT INTO businesses (name) VALUES ('Demo business') RETURNING id",
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO api_keys (business_id, key_hash, display_prefix) VALUES ($1, $2, $3)",
        )
        .bind(business_id)
        .bind(hash_key(api_key))
        .bind(&api_key[..8])
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO webhook_endpoints (business_id, url, signing_secret) VALUES ($1, $2, $3)",
        )
        .bind(business_id)
        .bind(url)
        .bind(secret)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
