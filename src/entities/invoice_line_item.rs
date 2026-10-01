use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Deserialize, Serialize, FromRow)]
#[serde(deny_unknown_fields)]
pub(crate) struct LineItem {
    pub(crate) description: String,
    pub(crate) quantity: i64,
    pub(crate) unit_amount_cents: i64,
}
