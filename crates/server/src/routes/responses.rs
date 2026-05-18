use axum::{
    Json,
    extract::{Query, State},
    response::IntoResponse,
};
use futures::StreamExt;
use mapper::MappingConfig;
use protocol::canonical::CanonicalRequest;
use protocol::chat::ChatCompletionRequest;
use protocol::error::ApiError;
use protocol::responses::ResponsesCreateRequest;
use protocol::sse::ResponseSseEvent;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::AppState;
use crate::config::SamplingConfig;
use crate::sse_writer;
use crate::store::{DebugArtifact, DebugArtifactView, ResponseState};

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
    tracing::info!(
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
        match state.store.get_canonical_messages(prev_id) {
            Ok(Some(messages)) => {
                history_messages = messages;
            }
            Ok(None) => {
                return Err(record_error(ApiError::response_not_found(prev_id)));
            }
            Err(error) => return Err(record_error(state_store_error(error))),
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
                let call_ids = missing_reasoning_call_ids(&mapped.chat_request.messages);
                let rc_map = state
                    .store
                    .find_reasoning_for_call_ids("default", &call_ids)
                    .map_err(|error| record_error(state_store_error(error)))?;
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
    // Apply config-level defaults before provider-specific normalization.
    let mut chat_req = mapped.chat_request.clone();
    apply_sampling_config(&mut chat_req, &state.config.sampling);
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

    let request_json = to_json_value("Responses request", &req).map_err(&record_error)?;
    let mapped_request_json =
        to_json_value("mapped Chat request", &chat_req).map_err(&record_error)?;
    let response_json =
        to_json_value("Responses object", &response_object).map_err(&record_error)?;
    let debug_annotations = debug_annotations_for_request(&request_json);
    persist_response_state(
        &state,
        ResponseState {
            conversation_id: None,
            id: mapped.response_id.clone(),
            model: resolved_model,
            created_at: response_object.created_at,
            status: response_object.status.clone(),
            response_json,
            previous_response_id: req.previous_response_id.clone(),
            canonical_messages,
        },
        DebugArtifact {
            conversation_id: None,
            id: mapped.response_id.clone(),
            model: response_object.model.clone(),
            created_at: response_object.created_at,
            status: response_object.status.clone(),
            request_json,
            mapped_request_json,
            upstream_error: None,
            debug_annotations,
            upstream_sse_events: vec![],
            response_sse_events: vec![],
        },
    )
    .map_err(&record_error)?;
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
    // Apply config-level defaults before provider-specific normalization.
    let mut chat_req = mapped.chat_request.clone();
    apply_sampling_config(&mut chat_req, &state.config.sampling);
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
    tracing::info!(%status, "Upstream response status");
    if status >= 400 {
        let body = upstream_resp.text().await.unwrap_or_default();
        tracing::warn!(%status, %body, "Upstream error response");
        let upstream_error = serde_json::from_str::<serde_json::Value>(&body)
            .unwrap_or_else(|_| serde_json::json!({ "body": body.clone() }));
        let request_json = to_json_value("Responses request", &_req).map_err(&record_error)?;
        let mapped_request_json =
            to_json_value("mapped Chat request", &chat_req).map_err(&record_error)?;
        let created_at = chrono::Utc::now().timestamp();
        persist_response_state(
            &state,
            ResponseState {
                conversation_id: None,
                id: mapped.response_id.clone(),
                model: resolved_model.clone(),
                created_at,
                status: "failed".into(),
                response_json: serde_json::Value::Null,
                previous_response_id: _req.previous_response_id.clone(),
                canonical_messages: chat_req.messages.clone(),
            },
            DebugArtifact {
                conversation_id: None,
                id: mapped.response_id.clone(),
                model: resolved_model,
                created_at,
                status: "failed".into(),
                upstream_error: Some(serde_json::json!({
                    "status": status,
                    "body": upstream_error,
                })),
                debug_annotations: debug_annotations_for_request(&request_json),
                request_json,
                mapped_request_json,
                upstream_sse_events: vec![],
                response_sse_events: vec![],
            },
        )
        .map_err(&record_error)?;
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
    let req_json = to_json_value("Responses request", &_req).map_err(&record_error)?;
    let sent_messages = chat_req.messages.clone();
    let previous_response_id = _req.previous_response_id.clone();

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
            mapper::custom_tools::custom_tool_names(_req.tools.as_deref().unwrap_or(&[])),
        );

        futures::pin_mut!(sse_stream);
        let mut response_body = serde_json::Value::Null;
        let mut upstream_sse_events: Vec<serde_json::Value> = Vec::new();
        let mut response_sse_events: Vec<serde_json::Value> = Vec::new();
        let mut chunk_count: u64 = 0;
        let mut text_bytes: u64 = 0;
        let mut final_events: Option<Vec<ResponseSseEvent>> = None;

        tracing::info!("Starting to read upstream SSE stream");

        while let Some(event_result) = sse_stream.next().await {
            match event_result {
                Ok(event) => {
                    let data = event.data;

                    if data == "[DONE]" {
                        upstream_sse_events.push(serde_json::json!({"data": "[DONE]"}));
                        let events = match stream_state.complete() {
                            Ok(events) => events,
                            Err(error) => {
                                let failed = ResponseSseEvent::ResponseFailed {
                                    response: {
                                        let mut response = protocol::sse::SseResponseShell::minimal(
                                            response_id.clone(),
                                            model.clone(),
                                            created_at,
                                        );
                                        response.status = "failed".into();
                                        response
                                    },
                                };
                                let error_event = ResponseSseEvent::Error { error: error.error };
                                for event in [error_event, failed] {
                                    if let Ok(event_json) = sse_event_to_value(&event) {
                                        response_sse_events.push(event_json);
                                    }
                                    let _ = tx.send(Ok(event)).await;
                                }
                                return;
                            }
                        };
                        for event in &events {
                            // Capture the completed response JSON for store
                            if let ResponseSseEvent::ResponseCompleted { response } = event {
                                response_body =
                                    match to_json_value("completed SSE response", response) {
                                        Ok(value) => value,
                                        Err(error) => {
                                            let _ = tx
                                                .send(Ok(ResponseSseEvent::Error {
                                                    error: error.error,
                                                }))
                                                .await;
                                            return;
                                        }
                                    };
                            }
                            let event_json = match sse_event_to_value(event) {
                                Ok(value) => value,
                                Err(error) => {
                                    let _ = tx
                                        .send(Ok(ResponseSseEvent::Error { error: error.error }))
                                        .await;
                                    return;
                                }
                            };
                            response_sse_events.push(event_json);
                        }
                        final_events = Some(events);
                        break;
                    }

                    upstream_sse_events.push(
                        serde_json::from_str::<serde_json::Value>(&data)
                            .unwrap_or_else(|_| serde_json::json!({ "data": data.clone() })),
                    );

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
                                        let event_json = match sse_event_to_value(&event) {
                                            Ok(value) => value,
                                            Err(error) => {
                                                let _ = tx
                                                    .send(Ok(ResponseSseEvent::Error {
                                                        error: error.error,
                                                    }))
                                                    .await;
                                                return;
                                            }
                                        };
                                        response_sse_events.push(event_json);
                                        if tx.send(Ok(event)).await.is_err() {
                                            return;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let event = ResponseSseEvent::Error { error: e.error };
                                    let event_json = match sse_event_to_value(&event) {
                                        Ok(value) => value,
                                        Err(error) => {
                                            let _ = tx
                                                .send(Ok(ResponseSseEvent::Error {
                                                    error: error.error,
                                                }))
                                                .await;
                                            return;
                                        }
                                    };
                                    response_sse_events.push(event_json);
                                    let _ = tx.send(Ok(event)).await;
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
                    let event = ResponseSseEvent::Error {
                        error: ApiError::stream_interrupted().error,
                    };
                    let event_json = match sse_event_to_value(&event) {
                        Ok(value) => value,
                        Err(error) => {
                            let _ = tx
                                .send(Ok(ResponseSseEvent::Error { error: error.error }))
                                .await;
                            return;
                        }
                    };
                    response_sse_events.push(event_json);
                    let _ = tx.send(Ok(event)).await;
                    return;
                }
            }
        }

        let reasoning_len = stream_state.reasoning_content.len();
        let usage = stream_state.final_usage();
        tracing::info!(
            %chunk_count, %text_bytes,
            has_tool = stream_state.tool_call_active,
            %reasoning_len,
            finish = ?stream_state.finish_reason,
            input_tokens = usage.map(|u| u.input_tokens),
            output_tokens = usage.map(|u| u.output_tokens),
            total_tokens = usage.map(|u| u.total_tokens),
            reasoning_tokens = usage.and_then(|u| u.output_tokens_details.as_ref().and_then(|d| d.reasoning_tokens)),
            "Stream completed"
        );

        // Build canonical messages from sent messages + synthesized assistant response
        let output_text = stream_state.accumulated_text.clone();
        let mut canonical = sent_messages;
        let has_content = !output_text.is_empty() || stream_state.tool_call_active;
        if has_content {
            let tool_calls = if stream_state.tool_call_active {
                Some(stream_state.chat_tool_calls())
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

        let debug_annotations = debug_annotations_for_request(&req_json);

        // Save to store for previous_response_id support
        let mapped_request_json = match serde_json::to_value(&chat_req) {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(%error, "Failed to serialize mapped Chat request");
                let _ = tx
                    .send(Ok(ResponseSseEvent::Error {
                        error: ApiError::internal(format!(
                            "failed to serialize mapped Chat request: {error}"
                        ))
                        .error,
                    }))
                    .await;
                return;
            }
        };
        if let Err(error) = store.put_response_state(ResponseState {
            conversation_id: None,
            id: response_id.clone(),
            model: model.clone(),
            created_at,
            status: "completed".into(),
            response_json: response_body,
            previous_response_id,
            canonical_messages: canonical,
        }) {
            tracing::error!(%error, "Failed to persist streamed response state");
            let _ = tx
                .send(Ok(ResponseSseEvent::Error {
                    error: ApiError::internal(format!(
                        "failed to persist streamed response state: {error}"
                    ))
                    .error,
                }))
                .await;
            return;
        }
        if let Err(error) = store.put_debug_artifact(DebugArtifact {
            conversation_id: None,
            id: response_id.clone(),
            model,
            created_at,
            status: "completed".into(),
            request_json: req_json,
            mapped_request_json,
            upstream_error: None,
            debug_annotations,
            upstream_sse_events,
            response_sse_events,
        }) {
            tracing::error!(%error, "Failed to persist streamed debug artifact");
        }
        if let Ok(size) = store.len() {
            metrics.set_store_size(size);
        }
        metrics.record_request_completed(stream_state.final_usage());

        if let Some(events) = final_events {
            for event in events {
                if tx.send(Ok(event)).await.is_err() {
                    return;
                }
            }
        }
    });

    let rx_stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    let sse = sse_writer::sse_response(rx_stream);
    Ok(sse.into_response())
}

pub async fn get_response(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    match state.store.get_response_json(&id) {
        Ok(Some(response_json)) => Ok(Json(response_json)),
        Ok(None) => Err(to_status_json(&ApiError::response_not_found(&id))),
        Err(error) => Err(to_status_json(&state_store_error(error))),
    }
}

pub async fn get_response_debug(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<DebugArtifactView>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    match state.store.get_debug_artifact(&id) {
        Ok(Some(artifact)) => Ok(Json(artifact)),
        Ok(None) => match state.store.get_response_json(&id) {
            Ok(Some(_)) => Err(to_status_json(&ApiError::debug_artifact_expired(&id))),
            Ok(None) => Err(to_status_json(&ApiError::response_not_found(&id))),
            Err(error) => Err(to_status_json(&state_store_error(error))),
        },
        Err(error) => Err(to_status_json(&state_store_error(error))),
    }
}

#[derive(Debug, Deserialize)]
pub struct DebugResponsesQuery {
    limit: Option<usize>,
}

pub async fn list_responses_debug(
    State(state): State<AppState>,
    Query(query): Query<DebugResponsesQuery>,
) -> Result<Json<Vec<DebugArtifactView>>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    let limit = query.limit.unwrap_or(20).clamp(1, 200);
    state
        .store
        .list_debug_artifacts(limit)
        .map(Json)
        .map_err(|error| to_status_json(&state_store_error(error)))
}

pub async fn delete_response(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, Json<serde_json::Value>)> {
    match state.store.delete(&id) {
        Ok(true) => {
            let size = state
                .store
                .len()
                .map_err(|error| to_status_json(&state_store_error(error)))?;
            state.metrics.set_store_size(size);
            Ok(Json(serde_json::json!({"id": id, "deleted": true})))
        }
        Ok(false) => Err(to_status_json(&ApiError::response_not_found(&id))),
        Err(error) => Err(to_status_json(&state_store_error(error))),
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
            Some("debug_artifact_expired") => StatusCode::GONE,
            _ => StatusCode::BAD_REQUEST,
        },
        "upstream_error" | "internal_error" => StatusCode::BAD_GATEWAY,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status, Json(serde_json::to_value(e).unwrap_or_default()))
}

fn missing_reasoning_call_ids(messages: &[protocol::chat::ChatMessage]) -> Vec<String> {
    let mut call_ids = Vec::new();
    for msg in messages {
        if let protocol::chat::ChatMessage::Assistant {
            reasoning_content: None,
            tool_calls: Some(tool_calls),
            ..
        } = msg
        {
            call_ids.extend(tool_calls.iter().map(|tool_call| tool_call.id.clone()));
        }
    }
    call_ids.sort();
    call_ids.dedup();
    call_ids
}

fn state_store_error(error: anyhow::Error) -> ApiError {
    ApiError::internal(format!("state store error: {error}"))
}

fn to_json_value<T: Serialize>(label: &str, value: &T) -> Result<serde_json::Value, ApiError> {
    serde_json::to_value(value)
        .map_err(|error| ApiError::internal(format!("failed to serialize {label}: {error}")))
}

fn sse_event_to_value(event: &ResponseSseEvent) -> Result<serde_json::Value, ApiError> {
    to_json_value("Responses SSE event", event)
}

fn apply_sampling_config(chat_req: &mut ChatCompletionRequest, sampling: &SamplingConfig) {
    if chat_req.temperature.is_none() {
        chat_req.temperature = sampling.temperature;
    }
    if chat_req.top_p.is_none() {
        chat_req.top_p = sampling.top_p;
    }
}

fn persist_response_state(
    state: &AppState,
    response_state: ResponseState,
    debug_artifact: DebugArtifact,
) -> Result<(), ApiError> {
    state
        .store
        .put_response_state(response_state)
        .map_err(state_store_error)?;
    state
        .store
        .put_debug_artifact(debug_artifact)
        .map_err(state_store_error)?;
    let size = state.store.len().map_err(state_store_error)?;
    state.metrics.set_store_size(size);
    Ok(())
}

fn debug_annotations_for_request(request: &serde_json::Value) -> Vec<String> {
    let mut annotations = Vec::new();
    let input_items = request
        .get("input")
        .and_then(|input| input.as_array())
        .or_else(|| request.get("items").and_then(|items| items.as_array()));
    let Some(items) = input_items else {
        return annotations;
    };

    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (idx, item) in items.iter().enumerate() {
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if item_type != "message" {
            continue;
        }
        let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if !matches!(role, "developer" | "system" | "user") {
            continue;
        }
        let key = serde_json::json!({
            "type": item_type,
            "role": role,
            "content": item.get("content").cloned().unwrap_or_default(),
        })
        .to_string();
        if let Some(first_idx) = seen.get(&key) {
            annotations.push(format!(
                "duplicate input message detected at indexes {first_idx} and {idx}; request was not modified"
            ));
            if annotations.len() >= 10 {
                annotations.push("duplicate input message scan stopped after 10 findings".into());
                break;
            }
        } else {
            seen.insert(key, idx);
        }
    }
    annotations
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
        let history_messages = state
            .store
            .get_canonical_messages(prev_id)
            .map_err(|error| record_error(state_store_error(error)))?
            .ok_or_else(|| record_error(ApiError::response_not_found(prev_id)))?;
        let mut input = body["input"].as_array().cloned().unwrap_or_default();
        // Prepend history messages as user/assistant messages
        for msg in history_messages.iter().rev() {
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
        let upstream_error = serde_json::from_str::<serde_json::Value>(&err_body)
            .unwrap_or_else(|_| serde_json::json!({ "body": err_body.clone() }));
        let request_json = to_json_value("canonical request", &canonical).map_err(&record_error)?;
        let response_id = format!("resp_{}", uuid::Uuid::new_v4());
        let created_at = chrono::Utc::now().timestamp();
        persist_response_state(
            &state,
            ResponseState {
                conversation_id: None,
                id: response_id.clone(),
                model: resolved_model.clone(),
                created_at,
                status: "failed".into(),
                response_json: serde_json::Value::Null,
                previous_response_id: canonical.previous_response_id.clone(),
                canonical_messages: canonical.clone().into_canonical_messages(),
            },
            DebugArtifact {
                conversation_id: None,
                id: response_id,
                model: resolved_model,
                created_at,
                status: "failed".into(),
                request_json: request_json.clone(),
                mapped_request_json: body.clone(),
                upstream_error: Some(serde_json::json!({
                    "status": status,
                    "body": upstream_error,
                })),
                debug_annotations: debug_annotations_for_request(&request_json),
                upstream_sse_events: vec![],
                response_sse_events: vec![],
            },
        )
        .map_err(&record_error)?;
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
        let req_json = to_json_value("canonical request", &canonical).map_err(&record_error)?;
        let previous_response_id = canonical.previous_response_id.clone();
        let resolved = resolved_model.clone();

        tokio::spawn(async move {
            use futures::StreamExt;
            let byte_stream = upstream_resp
                .bytes_stream()
                .map(|r| r.map_err(std::io::Error::other));
            let sse_stream = eventsource_stream::EventStream::new(byte_stream);
            futures::pin_mut!(sse_stream);

            let mut response_body = serde_json::Value::Null;
            let mut upstream_sse_events: Vec<serde_json::Value> = Vec::new();
            let mut response_sse_events: Vec<serde_json::Value> = Vec::new();
            let mut saw_completed = false;
            let mut completed_event: Option<protocol::sse::ResponseSseEvent> = None;
            while let Some(Ok(event)) = sse_stream.next().await {
                upstream_sse_events.push(
                    serde_json::from_str::<serde_json::Value>(&event.data)
                        .unwrap_or_else(|_| serde_json::json!({ "data": event.data.clone() })),
                );
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event.data) {
                    if parsed["type"] == "response.completed" {
                        response_body = parsed["response"].clone();
                        saw_completed = true;
                    }
                    if let Ok(sse_event) =
                        serde_json::from_value::<protocol::sse::ResponseSseEvent>(parsed.clone())
                    {
                        response_sse_events.push(parsed);
                        if matches!(
                            sse_event,
                            protocol::sse::ResponseSseEvent::ResponseCompleted { .. }
                        ) {
                            completed_event = Some(sse_event);
                        } else {
                            let _ = tx.send(Ok(sse_event)).await;
                        }
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
            let debug_annotations = debug_annotations_for_request(&req_json);

            let mapped_request_json = body.clone();
            if let Err(error) = store.put_response_state(ResponseState {
                conversation_id: None,
                id: store_id.clone(),
                model: resolved.clone(),
                created_at: response_body
                    .get("created_at")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| chrono::Utc::now().timestamp()),
                status: response_body
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("completed")
                    .into(),
                response_json: response_body.clone(),
                previous_response_id,
                canonical_messages: canonical_msgs_for_store,
            }) {
                tracing::error!(%error, "Failed to persist native streamed response state");
                let _ = tx
                    .send(Ok(protocol::sse::ResponseSseEvent::Error {
                        error: ApiError::internal(format!(
                            "failed to persist native streamed response state: {error}"
                        ))
                        .error,
                    }))
                    .await;
                return;
            }
            if let Err(error) = store.put_debug_artifact(DebugArtifact {
                conversation_id: None,
                id: store_id.clone(),
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
                mapped_request_json,
                upstream_error: None,
                debug_annotations,
                upstream_sse_events,
                response_sse_events,
            }) {
                tracing::error!(%error, "Failed to persist native streamed debug artifact");
            }
            if let Ok(size) = store.len() {
                metrics.set_store_size(size);
            }
            let usage = serde_json::from_value::<protocol::common::Usage>(
                response_body.get("usage").cloned().unwrap_or_default(),
            )
            .ok();
            metrics.record_request_completed(usage.as_ref());

            if let Some(event) = completed_event {
                let _ = tx.send(Ok(event)).await;
            }
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

        let request_json = to_json_value("canonical request", &canonical).map_err(&record_error)?;
        persist_response_state(
            &state,
            ResponseState {
                conversation_id: None,
                id: response_id.clone(),
                model: resolved_model.clone(),
                created_at,
                status: response_body["status"]
                    .as_str()
                    .unwrap_or("completed")
                    .into(),
                response_json: response_body.clone(),
                previous_response_id: canonical.previous_response_id.clone(),
                canonical_messages: canonical_msgs,
            },
            DebugArtifact {
                conversation_id: None,
                id: response_id.clone(),
                model: resolved_model,
                created_at,
                status: response_body["status"]
                    .as_str()
                    .unwrap_or("completed")
                    .into(),
                request_json: request_json.clone(),
                mapped_request_json: body.clone(),
                upstream_error: None,
                debug_annotations: debug_annotations_for_request(&request_json),
                upstream_sse_events: vec![],
                response_sse_events: vec![],
            },
        )
        .map_err(&record_error)?;
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
            "namespace" => {}
            "custom" => validate_custom_tool(item, idx)?,
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
        "function_call" | "local_shell_call" | "apply_patch_call" => Ok(()),
        "custom_tool_call" => validate_custom_tool_call(obj, path),
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

fn validate_custom_tool(item: &Value, idx: usize) -> Result<(), ApiError> {
    let format = item.get("format").ok_or_else(|| {
        ApiError::invalid_parameter(
            format!("tools[{idx}].format"),
            "missing_required_parameter",
            "custom tools must include a 'format' object",
        )
    })?;
    let format_obj = format.as_object().ok_or_else(|| {
        ApiError::invalid_parameter(
            format!("tools[{idx}].format"),
            "invalid_format",
            "custom tool format must be an object",
        )
    })?;
    for field in ["type", "syntax", "definition"] {
        if !format_obj.get(field).is_some_and(Value::is_string) {
            return Err(ApiError::invalid_parameter(
                format!("tools[{idx}].format.{field}"),
                "invalid_format",
                format!("custom tool format field '{field}' must be a string"),
            ));
        }
    }
    Ok(())
}

fn validate_custom_tool_call(obj: &Map<String, Value>, path: &str) -> Result<(), ApiError> {
    let input = obj.get("input").ok_or_else(|| {
        ApiError::invalid_parameter(
            format!("{path}.input"),
            "missing_required_parameter",
            "custom_tool_call input must be a string",
        )
    })?;
    if input.is_string() {
        Ok(())
    } else {
        Err(ApiError::invalid_parameter(
            format!("{path}.input"),
            "invalid_custom_tool_input",
            "custom_tool_call input must be a string",
        ))
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

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::chat::{ChatContent, ChatMessage};

    fn chat_request() -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "model".into(),
            messages: vec![ChatMessage::User {
                content: ChatContent::Text("hello".into()),
                name: None,
            }],
            stream: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            presence_penalty: None,
            frequency_penalty: None,
            stop: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            reasoning_effort: None,
            thinking: None,
            extra_body: serde_json::json!({}),
        }
    }

    #[test]
    fn debug_annotations_mark_duplicate_input_messages_without_rewriting() {
        let request = serde_json::json!({
            "input": [
                {"type": "message", "role": "developer", "content": "same"},
                {"type": "message", "role": "user", "content": "different"},
                {"type": "message", "role": "developer", "content": "same"}
            ]
        });

        let annotations = debug_annotations_for_request(&request);

        assert_eq!(annotations.len(), 1);
        assert!(annotations[0].contains("indexes 0 and 2"));
        assert_eq!(request["input"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn apply_sampling_config_fills_empty_sampling_fields() {
        let mut req = chat_request();
        let sampling = SamplingConfig {
            temperature: Some(0.2),
            top_p: Some(0.8),
        };

        apply_sampling_config(&mut req, &sampling);

        assert_eq!(req.temperature, Some(0.2));
        assert_eq!(req.top_p, Some(0.8));
    }

    #[test]
    fn apply_sampling_config_preserves_request_sampling_fields() {
        let mut req = chat_request();
        req.temperature = Some(0.7);
        req.top_p = Some(0.9);
        let sampling = SamplingConfig {
            temperature: Some(0.2),
            top_p: Some(0.8),
        };

        apply_sampling_config(&mut req, &sampling);

        assert_eq!(req.temperature, Some(0.7));
        assert_eq!(req.top_p, Some(0.9));
    }

    #[test]
    fn validate_tools_rejects_custom_without_format() {
        let tools = serde_json::json!([
            {
                "type": "custom",
                "name": "apply_patch",
                "description": "Use apply_patch"
            }
        ]);

        let err = validate_tools(&tools).unwrap_err();

        assert_eq!(err.error.param.as_deref(), Some("tools[0].format"));
        assert_eq!(
            err.error.code.as_deref(),
            Some("missing_required_parameter")
        );
    }

    #[test]
    fn validate_input_item_rejects_non_string_custom_tool_input() {
        let item = serde_json::json!({
            "type": "custom_tool_call",
            "call_id": "call_patch",
            "name": "apply_patch",
            "input": {"patch": "*** Begin Patch"}
        });

        let err = validate_input_item(&item, "input[0]").unwrap_err();

        assert_eq!(err.error.param.as_deref(), Some("input[0].input"));
        assert_eq!(err.error.code.as_deref(), Some("invalid_custom_tool_input"));
    }
}
