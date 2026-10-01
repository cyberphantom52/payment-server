use serde::Serialize;
use sqlx::FromRow;

#[derive(Debug, Serialize, FromRow)]
pub struct Business {
    pub id: i64,
    pub name: String,
}
