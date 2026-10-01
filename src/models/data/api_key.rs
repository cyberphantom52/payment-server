use crate::entities::api_key::KeyMetadata;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct IssuedApiKey {
    #[serde(flatten)]
    pub metadata: KeyMetadata,
    pub key: String,
}
