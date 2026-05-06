use axum::Json;
use serde_json::json;

pub async fn healthz() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}
