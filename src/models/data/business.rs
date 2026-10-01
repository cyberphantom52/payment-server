use crate::{entities::business::Business, models::data::api_key::IssuedApiKey};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CreatedBusiness {
    pub business: Business,
    pub api_key: IssuedApiKey,
}
