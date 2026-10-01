#[derive(sqlx::FromRow)]
pub(crate) struct Delivery {
    pub(crate) id: i64,
    pub(crate) payload: String,
    pub(crate) url: String,
    pub(crate) signing_secret: String,
    pub(crate) attempt_count: i64,
    pub(crate) remaining_ms: i64,
}
