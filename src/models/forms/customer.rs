use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewCustomer {
    pub(crate) name: String,
    pub(crate) email: String,
}
