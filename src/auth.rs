use crate::{AppState, error::AppError};
use axum::{
    extract::{FromRequestParts, Request, State},
    http::{header::AUTHORIZATION, request::Parts},
    middleware::Next,
    response::Response,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection, PgPool};
use std::fmt::Write;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BusinessId(pub i64);

impl<S: Send + Sync> FromRequestParts<S> for BusinessId {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Self>()
            .copied()
            .ok_or_else(AppError::unauthorized)
    }
}

pub async fn require_api_key(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let mut values = request.headers().get_all(AUTHORIZATION).iter();
    let header = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or_else(AppError::unauthorized)?;
    if values.next().is_some() {
        return Err(AppError::unauthorized());
    }
    let (scheme, key) = header.split_once(' ').ok_or_else(AppError::unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer")
        || key.len() != 64
        || !key.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::unauthorized());
    }
    let business_id = sqlx::query_scalar::<_, i64>(
        "SELECT business_id FROM api_keys WHERE key_hash = $1 AND revoked_at IS NULL",
    )
    .bind(hash_key(key))
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unauthorized)?;
    request.extensions_mut().insert(BusinessId(business_id));
    Ok(next.run(request).await)
}

#[derive(Debug, Serialize, FromRow)]
pub struct Business {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct KeyMetadata {
    pub id: i64,
    pub business_id: i64,
    pub display_prefix: String,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct IssuedApiKey {
    #[serde(flatten)]
    pub metadata: KeyMetadata,
    pub key: String,
}

#[derive(Debug, Serialize)]
pub struct CreatedBusiness {
    pub business: Business,
    pub api_key: IssuedApiKey,
}

pub async fn create_business(db: &PgPool, name: &str) -> Result<CreatedBusiness, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::invalid_input("Business name must not be empty"));
    }
    let mut transaction = db.begin().await?;
    let business = sqlx::query_as::<_, Business>(
        "INSERT INTO businesses (name) VALUES ($1) RETURNING id, name",
    )
    .bind(name)
    .fetch_one(&mut *transaction)
    .await?;
    let api_key = insert_key(&mut transaction, business.id).await?;
    transaction.commit().await?;
    Ok(CreatedBusiness { business, api_key })
}

pub async fn create_key(db: &PgPool, business_id: i64) -> Result<IssuedApiKey, AppError> {
    let mut connection = db.acquire().await?;
    require_business(&mut connection, business_id).await?;
    insert_key(&mut connection, business_id).await
}

pub async fn list_keys(db: &PgPool, business_id: i64) -> Result<Vec<KeyMetadata>, AppError> {
    let mut connection = db.acquire().await?;
    require_business(&mut connection, business_id).await?;
    Ok(sqlx::query_as("SELECT id, business_id, display_prefix, created_at, revoked_at FROM api_keys WHERE business_id = $1 ORDER BY id")
        .bind(business_id).fetch_all(&mut *connection).await?)
}

pub async fn revoke_key(db: &PgPool, key_id: i64) -> Result<KeyMetadata, AppError> {
    Ok(sqlx::query_as("UPDATE api_keys SET revoked_at = COALESCE(revoked_at, now()) WHERE id = $1 RETURNING id, business_id, display_prefix, created_at, revoked_at")
        .bind(key_id).fetch_one(db).await?)
}

async fn require_business(connection: &mut PgConnection, business_id: i64) -> Result<(), AppError> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM businesses WHERE id = $1)")
        .bind(business_id)
        .fetch_one(connection)
        .await?;
    if !exists {
        return Err(AppError::not_found());
    }
    Ok(())
}

async fn insert_key(
    connection: &mut PgConnection,
    business_id: i64,
) -> Result<IssuedApiKey, AppError> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(AppError::internal)?;
    let key = hex(&bytes);
    let metadata = sqlx::query_as("INSERT INTO api_keys (business_id, key_hash, display_prefix) VALUES ($1, $2, $3) RETURNING id, business_id, display_prefix, created_at, revoked_at")
        .bind(business_id).bind(hash_key(&key)).bind(&key[..8]).fetch_one(connection).await?;
    Ok(IssuedApiKey { metadata, key })
}

fn hash_key(key: &str) -> String {
    hex(&Sha256::digest(key.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
