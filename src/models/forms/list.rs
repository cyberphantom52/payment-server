use crate::error::AppError;
use serde::Deserialize;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListQuery {
    pub(crate) limit: Option<i64>,
    pub(crate) after_id: Option<i64>,
    pub(crate) state: Option<String>,
}

impl ListQuery {
    pub(crate) fn pagination(&self) -> Result<(i64, i64), AppError> {
        let limit = self.limit.unwrap_or(20);
        let after_id = self.after_id.unwrap_or(0);
        if !(1..=100).contains(&limit) || after_id < 0 {
            return Err(AppError::invalid_input(
                "limit must be 1..100 and after_id must be nonnegative",
            ));
        }
        Ok((limit, after_id))
    }
}
