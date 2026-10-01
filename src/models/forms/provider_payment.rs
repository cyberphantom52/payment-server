use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaymentRequest {
    pub(crate) invoice_id: i64,
    pub(crate) amount_cents: i64,
    pub(crate) card_token: String,
}
