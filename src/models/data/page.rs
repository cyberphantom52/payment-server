use serde::Serialize;

#[derive(Serialize)]
pub(crate) struct Page<T> {
    pub(crate) data: Vec<T>,
    pub(crate) next_after_id: Option<i64>,
}

pub(crate) fn page<T>(mut data: Vec<T>, limit: i64, id: impl Fn(&T) -> i64) -> Page<T> {
    let next_after_id = if data.len() > limit as usize {
        data.pop();
        data.last().map(id)
    } else {
        None
    };
    Page {
        data,
        next_after_id,
    }
}
