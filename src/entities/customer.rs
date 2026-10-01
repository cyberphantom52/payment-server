use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

#[derive(Serialize, FromRow)]
pub(crate) struct Customer {
    pub(crate) id: i64,
    pub(crate) business_id: i64,
    pub(crate) name: String,
    pub(crate) email: String,
    pub(crate) created_at: DateTime<Utc>,
}
