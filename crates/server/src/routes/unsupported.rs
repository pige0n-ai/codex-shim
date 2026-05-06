use axum::Json;

use protocol::error::ApiError;

pub async fn compact_not_implemented() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    to_status_json(&ApiError::endpoint_not_implemented(
        "OpenAI server-side /responses/compact is not implemented by codex-shim. \
         In the custom-provider path, Codex uses local compaction instead.",
    ))
}

pub async fn memories_not_implemented() -> (axum::http::StatusCode, Json<serde_json::Value>) {
    to_status_json(&ApiError::endpoint_not_implemented(
        "OpenAI server-side memory summarization is not implemented by codex-shim. \
         In the custom-provider path, Codex relies on local state and local compaction.",
    ))
}

fn to_status_json(e: &ApiError) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    (
        axum::http::StatusCode::BAD_REQUEST,
        Json(serde_json::to_value(e).unwrap_or_default()),
    )
}
