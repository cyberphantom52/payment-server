use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct ErrorBody {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}
#[derive(Serialize)]
pub(crate) struct Envelope {
    pub(crate) error: ErrorBody,
}
