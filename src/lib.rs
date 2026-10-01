pub mod auth;
pub mod error;
pub mod events;
pub mod payments;
pub mod provider;
pub mod resources;

use axum::{Json, Router, extract::State, middleware, routing::get};
use error::AppError;
use serde_json::{Value, json};
use sqlx::PgPool;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
}

pub fn router(state: AppState) -> Router {
    let protected =
        resources::router()
            .merge(payments::router())
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth::require_api_key,
            ));
    Router::new()
        .route("/health", get(health))
        .merge(protected)
        .fallback(|| async { AppError::not_found() })
        .method_not_allowed_fallback(|| async { AppError::method_not_allowed() })
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Result<Json<Value>, AppError> {
    sqlx::query("SELECT 1")
        .execute(&state.db)
        .await
        .map_err(|error| {
            eprintln!("database readiness error: {error}");
            AppError::unavailable("Database is unavailable")
        })?;
    Ok(Json(json!({ "status": "ok" })))
}
