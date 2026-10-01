#[derive(sqlx::FromRow)]
pub(crate) struct Operation {
    pub(crate) attempt_id: i64,
    pub(crate) invoice_id: i64,
    pub(crate) amount_cents: i64,
    pub(crate) mock_card_token: String,
    pub(crate) psp_ref: String,
    pub(crate) planned_outcome: String,
    pub(crate) ready_in_ms: i64,
}
