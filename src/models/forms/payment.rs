use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PaymentInput {
    pub(crate) card_token: String,
}
