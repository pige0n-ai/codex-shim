use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::{
    Json, Router,
    extract::State,
    response::{IntoResponse, Response, sse::Sse},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

// ── Captured request ─────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CapturedRequest {
    pub method: String,
    pub path: String,
    pub query_params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: serde_json::Value,
}

// ── Scenario ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Scenario {
    /// Chat non-stream: returns a single JSON response with given text.
    ChatNonStreamText {
        text: String,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Chat stream: returns SSE deltas.
    ChatStreamText {
        deltas: Vec<String>,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Chat stream: returns 429 for the first N requests, then streams deltas.
    ChatStream429ThenText {
        failures: usize,
        deltas: Vec<String>,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Chat stream: text chunk, delayed empty-choices usage chunk, then DONE.
    ChatStreamTextThenDelayedUsage {
        text: String,
        delay_ms: u64,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Chat stream with a tool call.
    ChatStreamToolCall {
        tool_name: String,
        tool_args: serde_json::Value,
        text_before: String,
        #[serde(default = "default_model")]
        model: String,
    },
    /// DeepSeek-style chat stream with reasoning_content before a tool call.
    ChatStreamReasoningToolCall {
        reasoning: String,
        tool_name: String,
        tool_args: serde_json::Value,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Chat stream with reasoning-only chunks separated by a delay.
    ChatStreamReasoningChunks {
        chunks: Vec<String>,
        delay_ms: u64,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Chat stream with a custom tool call whose arguments arrive in delayed chunks.
    ChatStreamCustomToolArgumentChunks {
        tool_name: String,
        argument_chunks: Vec<String>,
        delay_ms: u64,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Responses non-stream JSON response.
    ResponsesNonStream {
        response_id: String,
        output_text: String,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Responses stream: completed response.
    ResponsesStreamCompleted {
        response_id: String,
        output_text: String,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Responses stream: sends response.created then closes (abnormal end).
    ResponsesStreamAbnormalEnd {
        response_id: String,
        #[serde(default = "default_model")]
        model: String,
    },
    /// Upstream 401.
    Upstream401,
    /// Upstream 429.
    Upstream429,
}

fn default_model() -> String {
    "mock-model".into()
}

// ── State ────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MockUpstreamState {
    pub requests: Arc<Mutex<Vec<CapturedRequest>>>,
    pub scenario: Arc<Mutex<Scenario>>,
}

impl MockUpstreamState {
    pub fn new(scenario: Scenario) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            scenario: Arc::new(Mutex::new(scenario)),
        }
    }

    pub fn set_scenario(&self, s: Scenario) {
        *self.scenario.lock().unwrap() = s;
    }

    pub fn take_requests(&self) -> Vec<CapturedRequest> {
        std::mem::take(&mut *self.requests.lock().unwrap())
    }
}

// ── Server ───────────────────────────────────────────────────────

pub struct MockUpstream {
    pub state: MockUpstreamState,
    pub port: u16,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockUpstream {
    pub async fn start(scenario: Scenario) -> anyhow::Result<Self> {
        let state = MockUpstreamState::new(scenario);
        let app = build_router(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        Ok(Self {
            state,
            port,
            shutdown_tx: Some(shutdown_tx),
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }
}

impl Drop for MockUpstream {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

// ── Router ────────────────────────────────────────────────────────

fn build_router(state: MockUpstreamState) -> Router {
    Router::new()
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/responses", post(responses_create))
        .route("/v1/responses/{id}", get(responses_get))
        .route("/v1/models", get(models))
        .with_state(state)
}

// ── Chat Completions ─────────────────────────────────────────────

async fn chat_completions(
    State(state): State<MockUpstreamState>,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<HashMap<String, String>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let captured = CapturedRequest {
        method: "POST".into(),
        path: "/v1/chat/completions".into(),
        query_params: query.0.clone(),
        headers: headers
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect(),
        body: body.clone(),
    };
    let request_number = {
        let mut requests = state.requests.lock().unwrap();
        requests.push(captured);
        requests.len()
    };

    let scenario = state.scenario.lock().unwrap().clone();
    let stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match &scenario {
        Scenario::ChatNonStreamText { text, model } if !stream => {
            Json(serde_json::json!({
                "id": "chatcmpl_mock_1",
                "object": "chat.completion",
                "created": 1714771200,
                "model": model,
                "choices": [{"index": 0, "message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 5, "completion_tokens": 3, "total_tokens": 8}
            })).into_response()
        }
        Scenario::ChatNonStreamText { .. } => {
            // stream=true but scenario is non-stream — still return non-stream
            Json(serde_json::json!({
                "id": "chatcmpl_mock_1", "object": "chat.completion", "model": "mock-model",
                "choices": [{"index": 0, "message": {"role": "assistant", "content": "mock"}, "finish_reason": "stop"}]
            })).into_response()
        }
        Scenario::ChatStreamText { deltas, model } if stream => {
            sse_chat_stream(deltas, model)
        }
        Scenario::ChatStream429ThenText {
            failures,
            deltas,
            model,
        } if stream => {
            if request_number <= *failures {
                (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "0")],
                    Json(serde_json::json!({"error": {"message": "Rate limited", "type": "rate_limit_error", "code": "rate_limit_exceeded"}})),
                )
                    .into_response()
            } else {
                sse_chat_stream(deltas, model)
            }
        }
        Scenario::ChatStreamTextThenDelayedUsage {
            text,
            delay_ms,
            model,
        } if stream => sse_chat_stream_text_then_delayed_usage(text, *delay_ms, model),
        Scenario::ChatStreamText { .. } => {
            sse_chat_stream(&vec!["mock".into()], "mock-model")
        }
        Scenario::ChatStream429ThenText { .. } => {
            sse_chat_stream(&vec!["mock".into()], "mock-model")
        }
        Scenario::ChatStreamTextThenDelayedUsage { .. } => {
            sse_chat_stream(&vec!["mock".into()], "mock-model")
        }
        Scenario::ChatStreamToolCall { tool_name, tool_args, text_before, model } if stream => {
            sse_chat_stream_tool_call(tool_name, tool_args, text_before, model)
        }
        Scenario::ChatStreamToolCall { .. } => {
            Json(serde_json::json!({
                "id": "chatcmpl_mock", "object": "chat.completion", "model": "mock-model",
                "choices": [{"index":0,"message":{"role":"assistant","content":"tool"},"finish_reason":"stop"}]
            })).into_response()
        }
        Scenario::ChatStreamReasoningToolCall {
            reasoning,
            tool_name,
            tool_args,
            model,
        } if stream => sse_chat_stream_reasoning_tool_call(reasoning, tool_name, tool_args, model),
        Scenario::ChatStreamReasoningChunks {
            chunks,
            delay_ms,
            model,
        } if stream => sse_chat_stream_reasoning_chunks(chunks, *delay_ms, model),
        Scenario::ChatStreamCustomToolArgumentChunks {
            tool_name,
            argument_chunks,
            delay_ms,
            model,
        } if stream => {
            sse_chat_stream_custom_tool_argument_chunks(tool_name, argument_chunks, *delay_ms, model)
        }
        Scenario::ChatStreamReasoningToolCall { .. } => {
            Json(serde_json::json!({
                "id": "chatcmpl_mock", "object": "chat.completion", "model": "mock-model",
                "choices": [{"index":0,"message":{"role":"assistant","content":"tool"},"finish_reason":"stop"}]
            })).into_response()
        }
        Scenario::ChatStreamReasoningChunks { .. }
        | Scenario::ChatStreamCustomToolArgumentChunks { .. } => {
            sse_chat_stream(&vec!["mock".into()], "mock-model")
        }
        Scenario::Upstream401 => {
            (axum::http::StatusCode::UNAUTHORIZED,
             Json(serde_json::json!({"error": {"message": "Unauthorized", "type": "authentication_error", "code": "invalid_api_key"}}))
            ).into_response()
        }
        Scenario::Upstream429 => {
            (axum::http::StatusCode::TOO_MANY_REQUESTS,
             Json(serde_json::json!({"error": {"message": "Rate limited", "type": "rate_limit_error", "code": "rate_limit_exceeded"}}))
            ).into_response()
        }
        _ => {
            // Catch-all for responses-only scenarios hitting chat endpoint
            Json(serde_json::json!({
                "id": "chatcmpl_mock", "object": "chat.completion", "model": "mock-model",
                "choices": [{"index":0,"message":{"role":"assistant","content":"mock"},"finish_reason":"stop"}]
            })).into_response()
        }
    }
}

fn sse_chat_stream(deltas: &[String], model: &str) -> Response {
    let deltas: Vec<String> = deltas.to_vec();
    let model = model.to_string();

    let stream = async_stream::stream! {
        for (i, delta) in deltas.iter().enumerate() {
            let is_last = i == deltas.len() - 1;
            let finish: serde_json::Value = if is_last {
                serde_json::json!("stop")
            } else {
                serde_json::Value::Null
            };
            let chunk = serde_json::json!({
                "id": "chatcmpl_stream_1",
                "object": "chat.completion.chunk",
                "created": 1714771200,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": delta}, "finish_reason": finish}]
            });
            yield Ok::<_, std::convert::Infallible>(
                axum::response::sse::Event::default().data(serde_json::to_string(&chunk).unwrap())
            );
        }
        let usage_chunk = serde_json::json!({
            "id": "chatcmpl_stream_1", "object": "chat.completion.chunk", "created": 1714771200,
            "model": model, "choices": [],
            "usage": {"prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10}
        });
        yield Ok(axum::response::sse::Event::default().data(serde_json::to_string(&usage_chunk).unwrap()));
        yield Ok(axum::response::sse::Event::default().data("[DONE]"));
    };

    Sse::new(stream).into_response()
}

fn sse_chat_stream_text_then_delayed_usage(text: &str, delay_ms: u64, model: &str) -> Response {
    let text = text.to_string();
    let model = model.to_string();

    let stream = async_stream::stream! {
        let text_chunk = serde_json::json!({
            "id": "chatcmpl_delayed_usage_1",
            "object": "chat.completion.chunk",
            "created": 1714771200,
            "model": model,
            "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": null}]
        });
        yield Ok::<_, std::convert::Infallible>(
            axum::response::sse::Event::default().data(serde_json::to_string(&text_chunk).unwrap())
        );

        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let usage_chunk = serde_json::json!({
            "id": "chatcmpl_delayed_usage_1",
            "object": "chat.completion.chunk",
            "created": 1714771200,
            "model": model,
            "choices": [],
            "usage": {"prompt_tokens": 7, "completion_tokens": 11, "total_tokens": 18}
        });
        yield Ok(axum::response::sse::Event::default().data(serde_json::to_string(&usage_chunk).unwrap()));
        yield Ok(axum::response::sse::Event::default().data("[DONE]"));
    };

    Sse::new(stream).into_response()
}

fn sse_chat_stream_tool_call(
    tool_name: &str,
    tool_args: &serde_json::Value,
    text_before: &str,
    model: &str,
) -> Response {
    let tool_name = tool_name.to_string();
    let tool_args = tool_args.clone();
    let text_before = text_before.to_string();
    let model = model.to_string();

    let stream = async_stream::stream! {
        if !text_before.is_empty() {
            let chunk = serde_json::json!({
                "id": "chatcmpl_tool_1", "object": "chat.completion.chunk", "created": 1714771200,
                "model": model,
                "choices": [{"index": 0, "delta": {"content": text_before}, "finish_reason": null}]
            });
            yield Ok::<_, std::convert::Infallible>(
                axum::response::sse::Event::default().data(serde_json::to_string(&chunk).unwrap())
            );
        }
        let chunk = serde_json::json!({
            "id": "chatcmpl_tool_1", "object": "chat.completion.chunk", "created": 1714771200,
            "model": model,
            "choices": [{"index": 0, "delta": {
                "tool_calls": [{"index": 0, "id": "call_mock_1", "type": "function",
                    "function": {"name": tool_name, "arguments": serde_json::to_string(&tool_args).unwrap()}
                }]
            }, "finish_reason": null}]
        });
        yield Ok(axum::response::sse::Event::default().data(serde_json::to_string(&chunk).unwrap()));

        let final_chunk = serde_json::json!({
            "id": "chatcmpl_tool_1", "object": "chat.completion.chunk", "created": 1714771200,
            "model": model,
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
        });
        yield Ok(axum::response::sse::Event::default().data(serde_json::to_string(&final_chunk).unwrap()));
        yield Ok(axum::response::sse::Event::default().data("[DONE]"));
    };

    Sse::new(stream).into_response()
}

fn sse_chat_stream_reasoning_tool_call(
    reasoning: &str,
    tool_name: &str,
    tool_args: &serde_json::Value,
    model: &str,
) -> Response {
    let reasoning = reasoning.to_string();
    let tool_name = tool_name.to_string();
    let tool_args = tool_args.clone();
    let model = model.to_string();

    let stream = async_stream::stream! {
        let reasoning_chunk = serde_json::json!({
            "id": "chatcmpl_reasoning_tool_1",
            "object": "chat.completion.chunk",
            "created": 1714771200,
            "model": model,
            "choices": [{"index": 0, "delta": {"reasoning_content": reasoning}, "finish_reason": null}]
        });
        yield Ok::<_, std::convert::Infallible>(
            axum::response::sse::Event::default().data(serde_json::to_string(&reasoning_chunk).unwrap())
        );

        let tool_chunk = serde_json::json!({
            "id": "chatcmpl_reasoning_tool_1",
            "object": "chat.completion.chunk",
            "created": 1714771200,
            "model": model,
            "choices": [{"index": 0, "delta": {
                "tool_calls": [{"index": 0, "id": "call_mock_1", "type": "function",
                    "function": {"name": tool_name, "arguments": serde_json::to_string(&tool_args).unwrap()}
                }]
            }, "finish_reason": null}]
        });
        yield Ok(axum::response::sse::Event::default().data(serde_json::to_string(&tool_chunk).unwrap()));

        let final_chunk = serde_json::json!({
            "id": "chatcmpl_reasoning_tool_1",
            "object": "chat.completion.chunk",
            "created": 1714771200,
            "model": model,
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        yield Ok(axum::response::sse::Event::default().data(serde_json::to_string(&final_chunk).unwrap()));
        yield Ok(axum::response::sse::Event::default().data("[DONE]"));
    };

    Sse::new(stream).into_response()
}

fn sse_chat_stream_reasoning_chunks(chunks: &[String], delay_ms: u64, model: &str) -> Response {
    let chunks = chunks.to_vec();
    let model = model.to_string();

    let stream = async_stream::stream! {
        for (idx, reasoning) in chunks.iter().enumerate() {
            if idx > 0 && delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            let chunk = serde_json::json!({
                "id": "chatcmpl_reasoning_chunks_1",
                "object": "chat.completion.chunk",
                "created": 1714771200,
                "model": model,
                "choices": [{"index": 0, "delta": {"reasoning_content": reasoning}, "finish_reason": null}]
            });
            yield Ok::<_, std::convert::Infallible>(
                axum::response::sse::Event::default().data(serde_json::to_string(&chunk).unwrap())
            );
        }

        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        let final_chunk = serde_json::json!({
            "id": "chatcmpl_reasoning_chunks_1",
            "object": "chat.completion.chunk",
            "created": 1714771200,
            "model": model,
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        yield Ok(axum::response::sse::Event::default().data(serde_json::to_string(&final_chunk).unwrap()));
        yield Ok(axum::response::sse::Event::default().data("[DONE]"));
    };

    Sse::new(stream).into_response()
}

fn sse_chat_stream_custom_tool_argument_chunks(
    tool_name: &str,
    argument_chunks: &[String],
    delay_ms: u64,
    model: &str,
) -> Response {
    let tool_name = tool_name.to_string();
    let argument_chunks = argument_chunks.to_vec();
    let model = model.to_string();

    let stream = async_stream::stream! {
        for (idx, arguments) in argument_chunks.iter().enumerate() {
            if idx > 0 && delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            let function = if idx == 0 {
                serde_json::json!({"name": tool_name, "arguments": arguments})
            } else {
                serde_json::json!({"arguments": arguments})
            };
            let chunk = serde_json::json!({
                "id": "chatcmpl_custom_tool_chunks_1",
                "object": "chat.completion.chunk",
                "created": 1714771200,
                "model": model,
                "choices": [{"index": 0, "delta": {
                    "tool_calls": [{"index": 0, "id": "call_mock_1", "type": "function", "function": function}]
                }, "finish_reason": null}]
            });
            yield Ok::<_, std::convert::Infallible>(
                axum::response::sse::Event::default().data(serde_json::to_string(&chunk).unwrap())
            );
        }

        let final_chunk = serde_json::json!({
            "id": "chatcmpl_custom_tool_chunks_1",
            "object": "chat.completion.chunk",
            "created": 1714771200,
            "model": model,
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]
        });
        yield Ok(axum::response::sse::Event::default().data(serde_json::to_string(&final_chunk).unwrap()));
        yield Ok(axum::response::sse::Event::default().data("[DONE]"));
    };

    Sse::new(stream).into_response()
}

// ── Responses API ────────────────────────────────────────────────

async fn responses_create(
    State(state): State<MockUpstreamState>,
    headers: axum::http::HeaderMap,
    query: axum::extract::Query<HashMap<String, String>>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let captured = CapturedRequest {
        method: "POST".into(),
        path: "/v1/responses".into(),
        query_params: query.0.clone(),
        headers: headers
            .iter()
            .filter_map(|(k, v)| Some((k.as_str().to_string(), v.to_str().ok()?.to_string())))
            .collect(),
        body: body.clone(),
    };
    state.requests.lock().unwrap().push(captured);

    let scenario = state.scenario.lock().unwrap().clone();
    let stream = body
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match &scenario {
        Scenario::ResponsesNonStream { response_id, output_text, model } if !stream => {
            Json(serde_json::json!({
                "id": response_id,
                "object": "response",
                "created_at": 1714771200,
                "status": "completed",
                "model": model,
                "output": [{"type": "message", "id": "msg_mock_1", "status": "completed",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": output_text, "annotations": []}]}],
                "output_text": output_text,
                "usage": {"input_tokens": 5, "output_tokens": 3, "total_tokens": 8}
            })).into_response()
        }
        Scenario::ResponsesStreamCompleted { response_id, output_text, model } if stream => {
            sse_responses_stream_completed(response_id, output_text, model)
        }
        Scenario::ResponsesStreamAbnormalEnd { response_id, model } if stream => {
            sse_responses_stream_abnormal(response_id, model)
        }
        Scenario::ResponsesStreamCompleted { .. } | Scenario::ResponsesStreamAbnormalEnd { .. } => {
            Json(serde_json::json!({"id": "resp_mock", "object": "response", "status": "completed", "model": "mock-model", "output": []})).into_response()
        }
        Scenario::Upstream401 => {
            (axum::http::StatusCode::UNAUTHORIZED,
             Json(serde_json::json!({"error": {"message": "Unauthorized", "type": "authentication_error"}}))
            ).into_response()
        }
        Scenario::Upstream429 => {
            (axum::http::StatusCode::TOO_MANY_REQUESTS,
             Json(serde_json::json!({"error": {"message": "Rate limited", "type": "rate_limit_error"}}))
            ).into_response()
        }
        _ => {
            Json(serde_json::json!({
                "id": "resp_mock", "object": "response", "created_at": 1714771200,
                "status": "completed", "model": "mock-model",
                "output": [{"type": "message", "id": "msg_mock", "status": "completed",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "mock", "annotations": []}]}]
            })).into_response()
        }
    }
}

fn sse_responses_stream_completed(response_id: &str, output_text: &str, model: &str) -> Response {
    let response_id = response_id.to_string();
    let output_text = output_text.to_string();
    let model = model.to_string();

    let stream = async_stream::stream! {
        let created = serde_json::json!({
            "type": "response.created",
            "response": {"id": response_id, "object": "response", "created_at": 1714771200, "status": "in_progress", "model": model}
        });
        yield Ok::<_, std::convert::Infallible>(
            axum::response::sse::Event::default().event("response.created").data(serde_json::to_string(&created).unwrap())
        );
        let delta = serde_json::json!({
            "type": "response.output_text.delta", "item_id": "msg_1", "output_index": 0, "content_index": 0, "delta": output_text
        });
        yield Ok(axum::response::sse::Event::default().event("response.output_text.delta").data(serde_json::to_string(&delta).unwrap()));
        let completed = serde_json::json!({
            "type": "response.completed",
            "response": {
                "id": response_id, "object": "response", "created_at": 1714771200, "status": "completed", "model": model,
                "output": [{"type": "message", "id": "msg_1", "status": "completed", "role": "assistant",
                    "content": [{"type": "output_text", "text": output_text, "annotations": []}]}],
                "output_text": output_text,
                "usage": {"input_tokens": 1, "output_tokens": 1, "total_tokens": 2}
            }
        });
        yield Ok(axum::response::sse::Event::default().event("response.completed").data(serde_json::to_string(&completed).unwrap()));
    };

    Sse::new(stream).into_response()
}

fn sse_responses_stream_abnormal(response_id: &str, model: &str) -> Response {
    let response_id = response_id.to_string();
    let model = model.to_string();

    let stream = async_stream::stream! {
        let created = serde_json::json!({
            "type": "response.created",
            "response": {"id": response_id, "object": "response", "created_at": 1714771200, "status": "in_progress", "model": model}
        });
        yield Ok::<_, std::convert::Infallible>(
            axum::response::sse::Event::default().event("response.created").data(serde_json::to_string(&created).unwrap())
        );
        // Stream ends here — no response.completed
    };

    Sse::new(stream).into_response()
}

// ── GET /v1/responses/{id} ───────────────────────────────────────

async fn responses_get(
    State(state): State<MockUpstreamState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    let captured = CapturedRequest {
        method: "GET".into(),
        path: format!("/v1/responses/{id}"),
        query_params: HashMap::new(),
        headers: HashMap::new(),
        body: serde_json::Value::Null,
    };
    state.requests.lock().unwrap().push(captured);

    Json(serde_json::json!({
        "id": id, "object": "response", "status": "completed", "model": "mock-model",
        "output": [{"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "stored"}]}]
    })).into_response()
}

// ── GET /v1/models ───────────────────────────────────────────────

async fn models(State(state): State<MockUpstreamState>) -> Response {
    let captured = CapturedRequest {
        method: "GET".into(),
        path: "/v1/models".into(),
        query_params: HashMap::new(),
        headers: HashMap::new(),
        body: serde_json::Value::Null,
    };
    state.requests.lock().unwrap().push(captured);
    Json(serde_json::json!({"object": "list", "data": [{"id": "mock-model", "object": "model", "created": 1714771200, "owned_by": "mock"}]})).into_response()
}
