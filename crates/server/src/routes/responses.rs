use axum::{Json, extract::State, response::IntoResponse};
use futures::StreamExt;
use mapper::MappingConfig;
use protocol::canonical::CanonicalRequest;
use protocol::error::ApiError;
use protocol::responses::ResponsesCreateRequest;
use protocol::sse::ResponseSseEvent;
use serde_json::{Map, Value};

use crate::AppState;
use crate::sse_writer;
use crate::store::StoredResponse;

pub async fn create_response(
    State(state): State<AppState>,
    body: axum::extract::Json<serde_json::Value>,
) -> Result<axum::response::Response, (axum::http::StatusCode, Json<serde_json::Value>)> {
    state.metrics.record_request_received();
    let metrics = state.metrics.clone();
    let record_error = |e: ApiError| {
        metrics.record_request_error(e.error.message.clone());
        to_status_json(&e)
    };
    let root = body.0.as_object().ok_or_else(|| {
        record_error(ApiError::invalid_json("request body must be a JSON object"))
    })?;

    dump_debug_request_body(&body.0);

    validate_top_level_fields(root).map_err(&record_error)?;
    validate_raw_input(root.get("input")).map_err(&record_error)?;
    if let Some(include) = root.get("include") {
        validate_include(include).map_err(&record_error)?;
    }
    if let Some(tools) = root.get("tools") {
        validate_tools(tools).map_err(&record_error)?;
    }

    // Log request details for debugging
    let model = root.get("model").and_then(|v| v.as_str()).unwrap_or("?");
    let stream = root
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let prev_id = root
        .get("previous_response_id")
        .and_then(|v| v.as_str())
        .unwrap_or("-");
    let tool_count = root
        .get("tools")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let input_len = root
        .get("input")
        .and_then(|v| serde_json::to_string(v).ok())
        .map(|s| s.len())
        .unwrap_or(0);
    tracing::warn!(
        %model, %stream, %prev_id, %tool_count, %input_len,
        "Incoming request"
    );

    let req: ResponsesCreateRequest = serde_json::from_value(body.0).map_err(|e| {
        let msg = format!("Failed to deserialize request: {e}");
        tracing::warn!(error = %e, "Deserialization failure");
        record_error(ApiError::invalid_json(msg))
    })?;

    if req.model.is_empty() {
        return Err(record_error(ApiError::missing_model()));
    }

    let resolved_model = state.config.resolve_model(&req.model);

    let mapping_config = build_mapping_config(&state);

    // Handle previous_response_id: only require local store for non-stateful upstreams
    let mut history_messages = Vec::new();
    if let Some(ref prev_id) = req.previous_response_id
        && !state.profile.upstream_stateful()
    {
        match state.store.get(prev_id) {
            Some(record) => {
                history_messages = record.canonical_messages;
            }
            None => {
                return Err(record_error(ApiError::response_not_found(prev_id)));
            }
        }
    }
    // For stateful upstreams, previous_response_id will be forwarded directly
    // in the canonical request (handled by into_native_responses_json).

    // Parse to canonical IR
    let canonical = CanonicalRequest::from_request(&req, history_messages)
        .map_err(|e| record_error(ApiError::invalid_json(e)))?;

    // Validate canonical items against provider capabilities (fail-closed)
    protocol::canonical::validate_against_caps(&canonical, state.profile.capabilities())
        .map_err(record_error)?;

    // Log host tool warnings
    for warning in &canonical.host_tool_warnings {
        tracing::warn!(
            host_tool_warning = %warning,
            provider = %state.profile.kind(),
            "Hosted tool filtered"
        );
    }

    let caps = state.profile.capabilities();
    match caps.endpoint_mode {
        protocol::provider_caps::EndpointMode::ChatCompletionsShim => {
            let mut mapped = mapper::responses_to_chat_via_canonical(
                &canonical,
                state.profile.capabilities(),
                &mapping_config,
            )
            .map_err(|e| to_status_json(&e))?;

            state.profile.map_reasoning(&mut mapped.chat_request, &req);
            mapped.chat_request.model = resolved_model.clone();

            // Reasoning recovery for multi-turn tool calls
            let needs_recovery = canonical.needs_reasoning_recovery(state.profile.capabilities())
                || (!matches!(
                    state.profile.capabilities().reasoning_policy,
                    protocol::provider_caps::ReasoningPolicy::None
                ) && mapped.chat_request.messages.iter().any(|msg| {
                    matches!(
                        msg,
                        protocol::chat::ChatMessage::Assistant {
                            reasoning_content: None,
                            tool_calls: Some(_),
                            ..
                        }
                    )
                }));
            if needs_recovery {
                let rc_map = build_reasoning_map(&state.store);
                for msg in &mut mapped.chat_request.messages {
                    if let protocol::chat::ChatMessage::Assistant {
                        reasoning_content,
                        tool_calls: Some(tool_calls),
                        ..
                    } = msg
                        && reasoning_content.is_none()
                    {
                        for tc in tool_calls {
                            if let Some(rc) = rc_map.get(&tc.id) {
                                *reasoning_content = Some(rc.clone());
                                break;
                            }
                        }
                    }
                }
            }

            let stream = mapped.chat_request.stream.unwrap_or(false);
            if stream {
                handle_stream(state, mapped, req, resolved_model, mapping_config).await
            } else {
                handle_non_stream(state, mapped, req, resolved_model, mapping_config).await
            }
        }
        protocol::provider_caps::EndpointMode::NativeResponses => {
            handle_native_responses(state, canonical, resolved_model).await
        }
        protocol::provider_caps::EndpointMode::StatelessResponses => {
            handle_stateless_responses(state, canonical, resolved_model).await
        }
    }
}

fn dump_debug_request_body(body: &serde_json::Value) {
    let Ok(dir) = std::env::var("CODEX_SHIM_DEBUG_REQUEST_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let filename = format!(
        "{}-{}.json",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ"),
        uuid::Uuid::new_v4()
    );
    let path = dir.join(filename);
    let _ = std::fs::write(
        path,
        serde_json::to_string_pretty(body).unwrap_or_else(|_| body.to_string()),
    );
}

async fn handle_non_stream(
    state: AppState,
    mapped: mapper::MappedChatRequest,
    req: ResponsesCreateRequest,
    resolved_model: String,
    mapping_config: MappingConfig,
) -> Result<axum::response::Response, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let metrics = state.metrics.clone();
    let record_error = |e: ApiError| {
        metrics.record_request_error(e.error.message.clone());
        to_status_json(&e)
    };
    // Apply provider-specific normalization before sending
    let mut chat_req = mapped.chat_request.clone();
    let eb = state.profile.extra_body();
    if !eb.is_empty() {
        eb.merge_into(&mut chat_req.extra_body);
    }
    state.profile.pre_send(&mut chat_req);

    let chat_response = state
        .upstream
        .send_chat(&chat_req)
        .await
        .map_err(record_error)?;

    let response_object = mapper::response_mapper::map_chat_response_to_responses(
        &chat_response,
        &mapped.response_id,
        mapped.output_item_ids.first().unwrap_or(&String::new()),
        &req,
        &mapping_config,
    )
    .map_err(record_error)?;

    let canonical_messages =
        mapper::response_mapper::build_canonical_messages(&chat_req, &chat_response);

    state.store.put(StoredResponse {
        conversation_id: None,
        id: mapped.response_id.clone(),
        model: resolved_model,
        created_at: response_object.created_at,
        status: response_object.status.clone(),
        request_json: serde_json::to_value(&req).unwrap_or_default(),
        response_json: serde_json::to_value(&response_object).unwrap_or_default(),
        canonical_messages,
    });
    state.metrics.set_store_size(state.store.len());
    state
        .metrics
        .record_request_completed(response_object.usage.as_ref());

    Ok(Json(response_object).into_response())
}

async fn handle_stream(
    state: AppState,
    mapped: mapper::MappedChatRequest,
    _req: ResponsesCreateRequest,
    resolved_model: String,
    _mapping_config: MappingConfig,
) -> Result<axum::response::Response, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let metrics = state.metrics.clone();
    let record_error = |e: ApiError| {
        metrics.record_request_error(e.error.message.clone());
        to_status_json(&e)
    };
    // Apply provider-specific normalization before sending
    let mut chat_req = mapped.chat_request.clone();
    let eb = state.profile.extra_body();
    if !eb.is_empty() {
        eb.merge_into(&mut chat_req.extra_body);
    }
    state.profile.pre_send(&mut chat_req);

    let request_builder = state
        .upstream
        .build_request(reqwest::Method::POST, &state.config.upstream.chat_path)
        .await
        .map_err(record_error)?;

    let upstream_resp = request_builder
        .json(&chat_req)
        .send()
        .await
        .map_err(|e| record_error(ApiError::upstream_error(format!("{e}"))))?;

    let status = upstream_resp.status().as_u16();
    tracing::warn!(%status, "Upstream response status");
    if status >= 400 {
        let body = upstream_resp.text().await.unwrap_or_default();
        tracing::warn!(%status, %body, "Upstream error response");
        return Err(record_error(mapper::error_mapper::map_upstream_error(
            status, &body,
        )));
    }

    // Set up stream processing
    let response_id = mapped.response_id.clone();
    let output_item_id = mapped
        .output_item_ids
        .first()
        .cloned()
        .unwrap_or_else(|| format!("msg_{}", uuid::Uuid::new_v4()));
    let created_at = chrono::Utc::now().timestamp();
    let model = resolved_model.clone();

    let (tx, rx) =
        tokio::sync::mpsc::channel::<Result<ResponseSseEvent, std::convert::Infallible>>(64);

    // Capture what we need to save to store after streaming completes
    let store = state.store.clone();
    let metrics = state.metrics.clone();
    let req_json = serde_json::to_value(&_req).unwrap_or_default();
    let sent_messages = chat_req.messages.clone();

    // Spawn task to read upstream SSE and convert to Response events
    tokio::spawn(async move {
        let byte_stream = upstream_resp
            .bytes_stream()
            .map(|r| r.map_err(std::io::Error::other));

        let sse_stream = eventsource_stream::EventStream::new(byte_stream);
        let mut stream_state = mapper::sse_mapper::StreamState::new(
            response_id.clone(),
            model.clone(),
            created_at,
            output_item_id.clone(),
        );

        futures::pin_mut!(sse_stream);
        let mut response_body = serde_json::Value::Null;
        let mut chunk_count: u64 = 0;
        let mut text_bytes: u64 = 0;

        tracing::warn!("Starting to read upstream SSE stream");

        while let Some(event_result) = sse_stream.next().await {
            match event_result {
                Ok(event) => {
                    let data = event.data;

                    if data == "[DONE]" {
                        for event in stream_state.complete() {
                            // Capture the completed response JSON for store
                            if let ResponseSseEvent::ResponseCompleted { ref response } = event {
                                response_body = serde_json::to_value(response).unwrap_or_default();
                            }
                            let _ = tx.send(Ok(event)).await;
                        }
                        break;
                    }

                    match serde_json::from_str::<protocol::chat::ChatCompletionChunk>(&data) {
                        Ok(chunk) => {
                            chunk_count += 1;
                            if let Some(ref choices) = chunk.choices
                                && let Some(c) = choices.first()
                                && let Some(ref text) = c.delta.content
                            {
                                text_bytes += text.len() as u64;
                            }
                            match stream_state.process_chunk(&chunk) {
                                Ok(events) => {
                                    for event in events {
                                        if tx.send(Ok(event)).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = tx
                                        .send(Ok(ResponseSseEvent::Error { error: e.error }))
                                        .await;
                                    return;
                                }
                            }
                        }
                        Err(_) => {
                            tracing::debug!(%data, "Skipping unparseable SSE data");
                        }
                    }
                }
                Err(_e) => {
                    let _ = tx
                        .send(Ok(ResponseSseEvent::Error {
                            error: ApiError::stream_interrupted().error,
                        }))
                        .await;
                    return;
                }
            }
        }

        let reasoning_len = stream_state.reasoning_content.len();
        tracing::warn!(
            %chunk_count, %text_bytes,
            has_tool = stream_state.tool_call_active,
            %reasoning_len,
            finish = ?stream_state.finish_reason,
            "Stream completed"
        );

        // Build canonical messages from sent messages + synthesized assistant response
        let output_text = stream_state.accumulated_text.clone();
        let mut canonical = sent_messages;
        let has_content = !output_text.is_empty() || stream_state.tool_call_active;
        if has_content {
            let tool_calls = if stream_state.tool_call_active {
                Some(vec![protocol::chat::ChatToolCall {
                    id: stream_state.tool_call_id.clone().unwrap_or_default(),
                    call_type: "function".into(),
                    function: protocol::chat::ChatFunctionCall {
                        name: stream_state.tool_call_name.clone(),
                        arguments: stream_state.tool_call_arguments.clone(),
                    },
                }])
            } else {
                None
            };
            let text_content = if output_text.is_empty() {
                None
            } else {
                Some(protocol::chat::ChatContent::Text(output_text))
            };
            let reasoning = if stream_state.reasoning_content.is_empty() {
                None
            } else {
                Some(stream_state.reasoning_content.clone())
            };
            canonical.push(protocol::chat::ChatMessage::Assistant {
                content: text_content,
                name: None,
                tool_calls,
                reasoning_content: reasoning,
            });
        }

        // Save to store for previous_response_id support
        store.put(StoredResponse {
            conversation_id: None,
            id: response_id,
            model,
            created_at,
            status: "completed".into(),
            request_json: req_json,
            response_json: response_body,
            canonical_messages: canonical,
        });
        metrics.set_store_size(store.len());
        metrics.record_request_completed(stream_state.final_usage());
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let sse = sse_writer::sse_response(rx_stream);
    Ok(sse.into_response())
}

pub async fn get_response(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    match state.store.get(&id) {
        Some(record) => Ok(Json(record.response_json)),
        None => Err(to_status_json(&ApiError::response_not_found(&id))),
    }
}

pub async fn delete_response(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    if state.store.delete(&id) {
        state.metrics.set_store_size(state.store.len());
        Ok(Json(serde_json::json!({"id": id, "deleted": true})))
    } else {
        Err(to_status_json(&ApiError::response_not_found(&id)))
    }
}

fn to_status_json(e: &ApiError) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    use axum::http::StatusCode;

    let status = match e.error.error_type.as_str() {
        "upstream_auth_error" => StatusCode::UNAUTHORIZED,
        "upstream_rate_limited" => StatusCode::TOO_MANY_REQUESTS,
        "upstream_timeout" => StatusCode::GATEWAY_TIMEOUT,
        "not_implemented" => StatusCode::BAD_REQUEST,
        "stream_interrupted" => StatusCode::BAD_GATEWAY,
        "invalid_request_error" => match e.error.code.as_deref() {
            Some("response_not_found") => StatusCode::NOT_FOUND,
            _ => StatusCode::BAD_REQUEST,
        },
        "upstream_error" | "internal_error" => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::to_value(e).unwrap_or_default()))
}

/// Build a map of tool_call_id → reasoning_content from all stored responses.
fn build_reasoning_map(
    store: &crate::store::ResponseStore,
) -> std::collections::HashMap<String, String> {
    store.build_reasoning_map("default")
}

// ── Native Responses proxy ───────────────────────────────────────

async fn handle_native_responses(
    state: AppState,
    canonical: CanonicalRequest,
    resolved_model: String,
) -> Result<axum::response::Response, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let metrics = state.metrics.clone();
    let record_error = |e: ApiError| {
        metrics.record_request_error(e.error.message.clone());
        to_status_json(&e)
    };
    let mut body = canonical.into_native_responses_json();
    body["model"] = serde_json::Value::String(resolved_model.clone());

    // If the upstream is stateful, forward previous_response_id.
    // Otherwise, expand full history from store and inject into input.
    if !state.profile.upstream_stateful()
        && let Some(ref prev_id) = canonical.previous_response_id
    {
        let history = state
            .store
            .get(prev_id)
            .ok_or_else(|| record_error(ApiError::response_not_found(prev_id)))?;
        let mut input = body["input"].as_array().cloned().unwrap_or_default();
        // Prepend history messages as user/assistant messages
        for msg in history.canonical_messages.iter().rev() {
            let value = chat_message_to_responses_input(msg);
            input.insert(0, value);
        }
        body["input"] = serde_json::Value::Array(input);
    }

    let request_builder = state
        .upstream
        .build_request(reqwest::Method::POST, &state.config.upstream.responses_path)
        .await
        .map_err(record_error)?;

    let upstream_resp = request_builder
        .json(&body)
        .send()
        .await
        .map_err(|e| record_error(ApiError::upstream_error(format!("{e}"))))?;

    let status = upstream_resp.status().as_u16();
    if status >= 400 {
        let err_body = upstream_resp.text().await.unwrap_or_default();
        return Err(record_error(mapper::error_mapper::map_upstream_error(
            status, &err_body,
        )));
    }

    if canonical.stream {
        // SSE proxy
        let response_id = format!("resp_{}", uuid::Uuid::new_v4());
        let (tx, rx) = tokio::sync::mpsc::channel::<
            Result<protocol::sse::ResponseSseEvent, std::convert::Infallible>,
        >(64);
        let store = state.store.clone();
        let metrics = state.metrics.clone();
        let req_json = serde_json::to_value(&canonical).unwrap_or_default();
        let resolved = resolved_model.clone();

        tokio::spawn(async move {
            use futures::StreamExt;
            let byte_stream = upstream_resp
                .bytes_stream()
                .map(|r| r.map_err(std::io::Error::other));
            let sse_stream = eventsource_stream::EventStream::new(byte_stream);
            futures::pin_mut!(sse_stream);

            let mut response_body = serde_json::Value::Null;
            let mut saw_completed = false;
            while let Some(Ok(event)) = sse_stream.next().await {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event.data) {
                    if parsed["type"] == "response.completed" {
                        response_body = parsed["response"].clone();
                        saw_completed = true;
                    }
                    if let Ok(sse_event) =
                        serde_json::from_value::<protocol::sse::ResponseSseEvent>(parsed.clone())
                    {
                        let _ = tx.send(Ok(sse_event)).await;
                    }
                }
            }

            // Only write store if we saw a completed event
            if !saw_completed {
                tracing::error!(
                    "Native stream ended without response.completed; not writing store record"
                );
                return;
            }

            let store_id = response_body
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or(&response_id)
                .to_string();

            let canonical_msgs_for_store = {
                let request_msgs = canonical.into_canonical_messages();
                mapper::response_mapper::build_responses_canonical_messages(
                    &request_msgs,
                    &response_body,
                )
            };

            store.put(StoredResponse {
                conversation_id: None,
                id: store_id,
                model: resolved,
                created_at: response_body
                    .get("created_at")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| chrono::Utc::now().timestamp()),
                status: response_body
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("completed")
                    .into(),
                request_json: req_json,
                response_json: response_body.clone(),
                canonical_messages: canonical_msgs_for_store,
            });
            metrics.set_store_size(store.len());
            let usage = serde_json::from_value::<protocol::common::Usage>(
                response_body.get("usage").cloned().unwrap_or_default(),
            )
            .ok();
            metrics.record_request_completed(usage.as_ref());
        });

        let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        let sse = crate::sse_writer::sse_response(rx_stream);
        Ok(sse.into_response())
    } else {
        // Non-stream proxy
        let response_body: serde_json::Value = upstream_resp
            .json()
            .await
            .map_err(|e| record_error(ApiError::upstream_error(format!("{e}"))))?;

        let response_id = response_body["id"]
            .as_str()
            .unwrap_or("resp_unknown")
            .to_string();
        let created_at = response_body["created_at"]
            .as_i64()
            .unwrap_or(chrono::Utc::now().timestamp());

        let request_msgs = canonical.into_canonical_messages();
        let canonical_msgs = mapper::response_mapper::build_responses_canonical_messages(
            &request_msgs,
            &response_body,
        );

        state.store.put(StoredResponse {
            conversation_id: None,
            id: response_id.clone(),
            model: resolved_model,
            created_at,
            status: response_body["status"]
                .as_str()
                .unwrap_or("completed")
                .into(),
            request_json: serde_json::to_value(&canonical).unwrap_or_default(),
            response_json: response_body.clone(),
            canonical_messages: canonical_msgs,
        });
        state.metrics.set_store_size(state.store.len());
        let usage =
            serde_json::from_value::<protocol::common::Usage>(response_body["usage"].clone()).ok();
        state.metrics.record_request_completed(usage.as_ref());

        Ok(Json(response_body).into_response())
    }
}

// ── Stateless Responses proxy ────────────────────────────────────

async fn handle_stateless_responses(
    state: AppState,
    canonical: CanonicalRequest,
    resolved_model: String,
) -> Result<axum::response::Response, (axum::http::StatusCode, Json<serde_json::Value>)> {
    // Stateless is identical to native but always materializes full history
    handle_native_responses(state, canonical, resolved_model).await
}

// ── Helpers ──────────────────────────────────────────────────────

fn chat_message_to_responses_input(msg: &protocol::chat::ChatMessage) -> serde_json::Value {
    match msg {
        protocol::chat::ChatMessage::User { content, .. } => {
            let text = content_to_string(content);
            serde_json::json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]})
        }
        protocol::chat::ChatMessage::Assistant {
            content,
            tool_calls: _,
            reasoning_content: _,
            ..
        } => {
            let mut obj = serde_json::json!({"type": "message", "role": "assistant"});
            let text = content.as_ref().map(content_to_string).unwrap_or_default();
            obj["content"] = serde_json::json!([{"type": "output_text", "text": text}]);
            obj
        }
        protocol::chat::ChatMessage::System { content, .. } => {
            serde_json::json!({"type": "message", "role": "developer", "content": [{"type": "input_text", "text": content_to_string(content)}]})
        }
        protocol::chat::ChatMessage::Tool {
            content,
            tool_call_id,
        } => {
            serde_json::json!({"type": "function_call_output", "call_id": tool_call_id, "output": content_to_string(content)})
        }
    }
}

fn content_to_string(content: &protocol::chat::ChatContent) -> String {
    match content {
        protocol::chat::ChatContent::Text(s) => s.clone(),
        protocol::chat::ChatContent::Parts(parts) => parts
            .iter()
            .filter_map(|p| match p {
                protocol::chat::ChatContentPart::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(""),
    }
}
fn build_mapping_config(state: &AppState) -> MappingConfig {
    let caps = state.profile.capabilities();
    let is_chat_shim = matches!(
        caps.endpoint_mode,
        protocol::provider_caps::EndpointMode::ChatCompletionsShim
    );
    MappingConfig {
        thinking_enabled: is_chat_shim
            && !matches!(
                caps.reasoning_policy,
                protocol::provider_caps::ReasoningPolicy::None
            ),
        thinking_effort: None, // effort mapping is handled by provider.map_reasoning
        drop_sampling_params_when_thinking: state.profile.capabilities().reasoning_policy
            == protocol::provider_caps::ReasoningPolicy::DeepSeekReasoningContent,
        native_responses_passthrough: matches!(
            caps.endpoint_mode,
            protocol::provider_caps::EndpointMode::NativeResponses
                | protocol::provider_caps::EndpointMode::StatelessResponses
        ),
        provider_kind: state.profile.kind().to_string(),
    }
}

const ALLOWED_TOP_LEVEL_FIELDS: &[&str] = &[
    "model",
    "input",
    "instructions",
    "include",
    "client_metadata",
    "previous_response_id",
    "prompt_cache_key",
    "store",
    "stream",
    "max_output_tokens",
    "temperature",
    "top_p",
    "tools",
    "tool_choice",
    "parallel_tool_calls",
    "reasoning",
    "text",
    "metadata",
];

const KNOWN_UNIMPLEMENTED_TOP_LEVEL_FIELDS: &[&str] = &[
    "conversation",
    "background",
    "truncation",
    "max_tool_calls",
    "model_verbosity",
];

const ALLOWED_INCLUDE_FIELDS: &[&str] = &[];

fn validate_top_level_fields(body: &Map<String, Value>) -> Result<(), ApiError> {
    for key in body.keys() {
        if KNOWN_UNIMPLEMENTED_TOP_LEVEL_FIELDS.contains(&key.as_str()) {
            return Err(ApiError::field_not_implemented(key));
        }
        if !ALLOWED_TOP_LEVEL_FIELDS.contains(&key.as_str()) {
            return Err(ApiError::unknown_parameter(key));
        }
    }
    Ok(())
}

fn validate_include(include: &Value) -> Result<(), ApiError> {
    let values = include.as_array().ok_or_else(|| {
        ApiError::invalid_parameter(
            "include",
            "invalid_include",
            "include must be an array of strings",
        )
    })?;
    for value in values {
        let entry = value.as_str().ok_or_else(|| {
            ApiError::invalid_parameter(
                "include",
                "invalid_include",
                "include must contain only strings",
            )
        })?;
        if !ALLOWED_INCLUDE_FIELDS.contains(&entry) {
            return Err(ApiError::unsupported_include(entry));
        }
    }
    Ok(())
}

fn validate_tools(tools: &Value) -> Result<(), ApiError> {
    let items = tools.as_array().ok_or_else(|| {
        ApiError::invalid_parameter("tools", "invalid_tools", "tools must be an array")
    })?;
    for (idx, item) in items.iter().enumerate() {
        let item_type = item.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
            ApiError::invalid_parameter(
                format!("tools[{idx}]"),
                "missing_type",
                "tool entries must include a string 'type' field",
            )
        })?;
        match item_type {
            "function" => {}
            "namespace" => {},
            "web_search" | "web_search_preview" => {
                return Err(ApiError::hosted_tool_not_supported("web_search"));
            }
            "file_search" => {
                return Err(ApiError::hosted_tool_not_supported("file_search"));
            }
            "code_interpreter" => {
                return Err(ApiError::hosted_tool_not_supported("code_interpreter"));
            }
            "computer_use" => {
                return Err(ApiError::hosted_tool_not_supported("computer_use"));
            }
            "mcp" => {
                return Err(ApiError::hosted_tool_not_supported("mcp"));
            }
            other => {
                return Err(ApiError::unsupported_tool_type(other, idx));
            }
        }
    }
    Ok(())
}

fn validate_raw_input(input: Option<&Value>) -> Result<(), ApiError> {
    let Some(input) = input else {
        return Ok(());
    };
    match input {
        Value::String(_) => Ok(()),
        Value::Object(_) => validate_input_item(input, "input"),
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                validate_input_item(item, &format!("input[{idx}]"))?;
            }
            Ok(())
        }
        _ => Err(ApiError::invalid_parameter(
            "input",
            "invalid_input",
            "input must be a string, a Responses item object, or an array of Responses items",
        )),
    }
}

fn validate_input_item(item: &Value, path: &str) -> Result<(), ApiError> {
    let obj = item.as_object().ok_or_else(|| {
        ApiError::invalid_parameter(
            path,
            "invalid_input_item",
            "input items must be JSON objects",
        )
    })?;
    let item_type = obj.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
        ApiError::invalid_parameter(
            path,
            "missing_type",
            "input items must include a string 'type' field",
        )
    })?;
    match item_type {
        "message" => validate_message_item(obj, path),
        "function_call" | "custom_tool_call" | "local_shell_call" | "apply_patch_call" => Ok(()),
        "function_call_output" => validate_function_call_output(obj, path),
        "custom_tool_call_output" | "local_shell_call_output" | "apply_patch_call_output" => Ok(()),
        "reasoning" => validate_reasoning_item(obj, path),
        "mcp_call" => Err(ApiError::unsupported_input_item("mcp_call")),
        "web_search_call" => Err(ApiError::unsupported_input_item("web_search_call")),
        "file_search_call" => Err(ApiError::unsupported_input_item("file_search_call")),
        "code_interpreter_call" => Err(ApiError::unsupported_input_item("code_interpreter_call")),
        "computer_call" => Err(ApiError::unsupported_input_item("computer_call")),
        "input_file" => Err(ApiError::file_input_not_supported()),
        other => Err(ApiError::unknown_input_item(Some(other))),
    }
}

fn validate_message_item(obj: &Map<String, Value>, path: &str) -> Result<(), ApiError> {
    let Some(content) = obj.get("content") else {
        return Ok(());
    };
    match content {
        Value::String(_) => Ok(()),
        Value::Array(parts) => {
            for (idx, part) in parts.iter().enumerate() {
                validate_content_part(part, &format!("{path}.content[{idx}]"))?;
            }
            Ok(())
        }
        _ => Err(ApiError::invalid_parameter(
            path,
            "invalid_message_content",
            "message content must be a string or an array of content parts",
        )),
    }
}

fn validate_function_call_output(obj: &Map<String, Value>, path: &str) -> Result<(), ApiError> {
    let Some(output) = obj.get("output") else {
        return Ok(());
    };
    match output {
        Value::String(_) => Ok(()),
        Value::Array(parts) => {
            for (idx, part) in parts.iter().enumerate() {
                let part_type = part.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
                    ApiError::invalid_parameter(
                        format!("{path}.output[{idx}]"),
                        "missing_type",
                        "function_call_output parts must include a string 'type' field",
                    )
                })?;
                if part_type != "input_text" {
                    return Err(ApiError::unsupported_content_part(part_type));
                }
                if part.get("text").and_then(|v| v.as_str()).is_none() {
                    return Err(ApiError::invalid_parameter(
                        format!("{path}.output[{idx}]"),
                        "invalid_output_part",
                        "function_call_output input_text parts must include string text",
                    ));
                }
            }
            Ok(())
        }
        _ => Err(ApiError::invalid_parameter(
            path,
            "invalid_function_call_output",
            "function_call_output.output must be a string or an array of input_text parts",
        )),
    }
}

fn validate_reasoning_item(obj: &Map<String, Value>, path: &str) -> Result<(), ApiError> {
    if let Some(content) = obj.get("content") {
        let parts = content.as_array().ok_or_else(|| {
            ApiError::invalid_parameter(
                format!("{path}.content"),
                "invalid_reasoning_content",
                "reasoning.content must be an array",
            )
        })?;
        for (idx, part) in parts.iter().enumerate() {
            let part_type = part.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
                ApiError::invalid_parameter(
                    format!("{path}.content[{idx}]"),
                    "missing_type",
                    "reasoning content parts must include a string 'type' field",
                )
            })?;
            if part_type != "output_text" {
                return Err(ApiError::unsupported_content_part(part_type));
            }
        }
    }
    if let Some(summary) = obj.get("summary") {
        let parts = summary.as_array().ok_or_else(|| {
            ApiError::invalid_parameter(
                format!("{path}.summary"),
                "invalid_reasoning_summary",
                "reasoning.summary must be an array",
            )
        })?;
        for (idx, part) in parts.iter().enumerate() {
            let part_type = part.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
                ApiError::invalid_parameter(
                    format!("{path}.summary[{idx}]"),
                    "missing_type",
                    "reasoning summary parts must include a string 'type' field",
                )
            })?;
            if part_type != "summary_text" {
                return Err(ApiError::invalid_parameter(
                    format!("{path}.summary[{idx}]"),
                    "unsupported_reasoning_summary",
                    format!("reasoning summary part type '{part_type}' is not supported"),
                ));
            }
        }
    }
    Ok(())
}

fn validate_content_part(part: &Value, path: &str) -> Result<(), ApiError> {
    let part_type = part.get("type").and_then(|v| v.as_str()).ok_or_else(|| {
        ApiError::invalid_parameter(
            path,
            "missing_type",
            "content parts must include a string 'type' field",
        )
    })?;
    match part_type {
        "input_text" | "input_image" | "output_text" => Ok(()),
        "refusal" => Err(ApiError::unsupported_content_part("refusal")),
        other => Err(ApiError::unsupported_content_part(other)),
    }
}
