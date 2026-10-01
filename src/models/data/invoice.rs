use crate::entities::{invoice::InvoiceSummary, invoice_line_item::LineItem};
use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct Invoice {
    #[serde(flatten)]
    pub(crate) summary: InvoiceSummary,
    pub(crate) line_items: Vec<LineItem>,
}
