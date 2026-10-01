use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::FromRow;

#[derive(Serialize, FromRow)]
pub(crate) struct Attempt {
    pub(crate) id: i64,
    pub(crate) business_id: i64,
    pub(crate) invoice_id: i64,
    #[serde(rename = "status")]
    pub(crate) state: String,
    pub(crate) psp_ref: Option<String>,
    pub(crate) failure_code: Option<String>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(FromRow)]
pub(crate) struct Replay {
    pub(crate) id: i64,
    pub(crate) invoice_id: i64,
    pub(crate) mock_card_token: String,
    pub(crate) accepted_response_body: String,
}

#[derive(FromRow)]
pub(crate) struct Work {
    pub(crate) id: i64,
    pub(crate) business_id: i64,
    pub(crate) invoice_id: i64,
    pub(crate) mock_card_token: String,
    pub(crate) total_cents: i64,
}
