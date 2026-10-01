use crate::entities::invoice_line_item::LineItem;
use chrono::NaiveDate;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewInvoice {
    pub(crate) customer_id: i64,
    pub(crate) due_date: NaiveDate,
    pub(crate) line_items: Vec<LineItem>,
}
