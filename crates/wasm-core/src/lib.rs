use mapper::MappingConfig;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;

/// Map a Responses API request JSON string to a Chat Completions request JSON string.
#[wasm_bindgen]
pub fn map_responses_request(request_json: &str, config_json: &str) -> Result<String, JsValue> {
    let req: protocol::responses::ResponsesCreateRequest =
        serde_json::from_str(request_json).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let config: WasmMappingConfig =
        serde_json::from_str(config_json).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let mapping_config = MappingConfig {
        thinking_enabled: config.thinking_enabled,
        thinking_effort: config.thinking_effort,
        drop_sampling_params_when_thinking: config.drop_sampling_params_when_thinking,
        native_responses_passthrough: config.native_responses_passthrough,
        provider_kind: config.provider_kind,
    };

    let result = mapper::responses_to_chat(&req, &[], &mapping_config)
        .map_err(|e| JsValue::from_str(&e.error.message))?;

    serde_json::to_string(&result.chat_request).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Map a Chat Completions response JSON string to a Responses API object JSON string.
#[wasm_bindgen]
pub fn map_chat_response(
    response_json: &str,
    request_json: &str,
    config_json: &str,
) -> Result<String, JsValue> {
    let chat: protocol::chat::ChatCompletionResponse =
        serde_json::from_str(response_json).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let req: protocol::responses::ResponsesCreateRequest =
        serde_json::from_str(request_json).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let config: WasmMappingConfig =
        serde_json::from_str(config_json).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let mapping_config = MappingConfig {
        thinking_enabled: config.thinking_enabled,
        thinking_effort: config.thinking_effort,
        drop_sampling_params_when_thinking: config.drop_sampling_params_when_thinking,
        native_responses_passthrough: config.native_responses_passthrough,
        provider_kind: config.provider_kind,
    };

    let response_id = format!("resp_{}", uuid::Uuid::new_v4());
    let item_id = format!("msg_{}", uuid::Uuid::new_v4());

    let result = mapper::response_mapper::map_chat_response_to_responses(
        &chat,
        &response_id,
        &item_id,
        &req,
        &mapping_config,
    )
    .map_err(|e| JsValue::from_str(&e.error.message))?;

    serde_json::to_string(&result).map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Process a Chat SSE chunk and return the corresponding Response SSE events as JSON.
#[wasm_bindgen]
pub fn process_sse_chunk(chunk_json: &str, state_json: &str) -> Result<String, JsValue> {
    let chunk: protocol::chat::ChatCompletionChunk =
        serde_json::from_str(chunk_json).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let state: SseState =
        serde_json::from_str(state_json).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let mut stream_state = mapper::sse_mapper::StreamState::new(
        state.response_id,
        state.model,
        state.created_at,
        state.output_item_id,
    );

    let events = stream_state
        .process_chunk(&chunk)
        .map_err(|e| JsValue::from_str(&e.error.message))?;

    serde_json::to_string(&events).map_err(|e| JsValue::from_str(&e.to_string()))
}

#[derive(Serialize, Deserialize)]
struct WasmMappingConfig {
    #[serde(default)]
    thinking_enabled: bool,
    #[serde(default)]
    thinking_effort: Option<String>,
    #[serde(default)]
    drop_sampling_params_when_thinking: bool,
    #[serde(default)]
    native_responses_passthrough: bool,
    #[serde(default = "default_provider_kind")]
    provider_kind: String,
}

fn default_provider_kind() -> String {
    "generic-openai-chat".into()
}

#[derive(Serialize, Deserialize)]
struct SseState {
    response_id: String,
    model: String,
    created_at: i64,
    output_item_id: String,
}
