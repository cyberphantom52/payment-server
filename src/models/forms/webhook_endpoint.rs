use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EndpointRequest {
    pub(crate) url: String,
}
