use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Event {
    pub(crate) id: i64,
    #[serde(rename = "type")]
    pub(crate) event_type: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) data: Value,
}
