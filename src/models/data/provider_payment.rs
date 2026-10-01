use crate::models::enums::failure_code::FailureCode;
use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(tag = "status", deny_unknown_fields)]
pub(crate) enum ProviderResult {
    #[serde(rename = "pending")]
    Pending { attempt_id: i64 },
    #[serde(rename = "succeeded")]
    Succeeded { attempt_id: i64, psp_ref: Uuid },
    #[serde(rename = "failed")]
    Failed { attempt_id: i64, code: FailureCode },
}

impl ProviderResult {
    pub(crate) fn attempt_id(&self) -> i64 {
        match self {
            Self::Pending { attempt_id }
            | Self::Succeeded { attempt_id, .. }
            | Self::Failed { attempt_id, .. } => *attempt_id,
        }
    }
}
