use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FailureCode {
    CardDeclined,
    InsufficientFunds,
}
