use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;
use sqlx::FromRow;

#[derive(Serialize, FromRow)]
pub(crate) struct InvoiceSummary {
    pub(crate) id: i64,
    pub(crate) business_id: i64,
    pub(crate) customer_id: i64,
    pub(crate) due_date: NaiveDate,
    pub(crate) total_cents: i64,
    pub(crate) state: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) paid_at: Option<DateTime<Utc>>,
}
