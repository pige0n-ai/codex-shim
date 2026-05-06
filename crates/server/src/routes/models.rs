use axum::{Json, extract::State};

use protocol::models::{ModelsResponse, build_model_catalog};

use crate::AppState;

pub async fn models(
    State(state): State<AppState>,
) -> Result<Json<ModelsResponse>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let catalog = build_model_catalog(&state.config.models.catalog, state.profile.capabilities());
    Ok(Json(catalog))
}
