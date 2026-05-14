use protocol::chat::{ChatCompletionResponse, ChatMessage};
use protocol::common::ContentPart;
use protocol::error::ApiError;
use protocol::responses::{
    ResponseOutputItem, ResponsesCreateRequest, ResponsesObject, SummaryPart,
};

use crate::MappingConfig;
use crate::tool_call_normalizer::normalize_chat_tool_calls;

/// Map an upstream Chat Completions response back to a Responses API object.
pub fn map_chat_response_to_responses(
    chat: &ChatCompletionResponse,
    response_id: &str,
    output_item_id: &str,
    original_req: &ResponsesCreateRequest,
    _config: &MappingConfig,
) -> Result<ResponsesObject, ApiError> {
    let choices = chat.choices.as_deref().unwrap_or(&[]);

    if choices.is_empty() {
        return Err(ApiError::upstream_error("Upstream returned empty choices"));
    }

    let choice = &choices[0];
    let msg = &choice.message;
    let mut output: Vec<ResponseOutputItem> = Vec::new();

    // 1. Emit reasoning item if reasoning_content is present.
    // Codex requires `summary` (Vec<ReasoningItemReasoningSummary>), not `content`
    // with `output_text` (which isn't a recognized ReasoningItemContent variant).
    if let Some(rc) = &msg.reasoning_content
        && !rc.is_empty()
    {
        output.push(ResponseOutputItem::Reasoning {
            id: format!("rs_{}", uuid::Uuid::new_v4()),
            content: None,
            summary: Some(vec![SummaryPart::SummaryText { text: rc.clone() }]),
            status: Some("completed".into()),
        });
    }

    // 2. Emit tool call items
    if let Some(tool_calls) = &msg.tool_calls {
        for tc in normalize_chat_tool_calls(tool_calls) {
            let fn_name = tc.function.name.clone().unwrap_or_default();
            output.push(ResponseOutputItem::FunctionCall {
                id: format!("fc_{}", uuid::Uuid::new_v4()),
                status: "completed".into(),
                call_id: tc.id.clone(),
                name: fn_name,
                arguments: tc.function.arguments.clone(),
            });
        }
    }

    // 3. Emit message item (if there's text content, or no tool calls)
    let text_content = extract_text_content(&msg.content);
    let output_text = if text_content.is_some() || output.is_empty() {
        let text = text_content.clone().unwrap_or_default();
        output.push(ResponseOutputItem::Message {
            id: output_item_id.to_string(),
            status: "completed".into(),
            role: "assistant".into(),
            content: vec![ContentPart::OutputText {
                text: text.clone(),
                annotations: vec![],
            }],
        });
        text
    } else {
        text_content.unwrap_or_default()
    };

    Ok(ResponsesObject {
        id: response_id.to_string(),
        object: "response".into(),
        created_at: chat.created,
        status: "completed".into(),
        status_details: None,
        model: chat.model.clone(),
        output,
        output_text: Some(output_text),
        usage: chat.usage.clone(),
        metadata: original_req.metadata.clone(),
        previous_response_id: None,
    })
}

fn extract_text_content(content: &Option<protocol::chat::ChatContent>) -> Option<String> {
    match content.as_ref()? {
        protocol::chat::ChatContent::Text(t) => {
            if t.is_empty() {
                None
            } else {
                Some(t.clone())
            }
        }
        protocol::chat::ChatContent::Parts(parts) => {
            let text: String = parts
                .iter()
                .filter_map(|p| match p {
                    protocol::chat::ChatContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            if text.is_empty() { None } else { Some(text) }
        }
    }
}

/// Build the canonical Chat message list from a request + response,
/// to store for `previous_response_id` continuations.
pub fn build_canonical_messages(
    chat_request: &protocol::chat::ChatCompletionRequest,
    chat_response: &ChatCompletionResponse,
) -> Vec<ChatMessage> {
    let mut messages = chat_request.messages.clone();

    // Add the assistant's response
    if let Some(choices) = &chat_response.choices
        && let Some(choice) = choices.first()
    {
        let msg = &choice.message;
        messages.push(ChatMessage::Assistant {
            content: msg.content.clone(),
            name: None,
            tool_calls: msg
                .tool_calls
                .as_ref()
                .map(|tool_calls| normalize_chat_tool_calls(tool_calls)),
            reasoning_content: msg.reasoning_content.clone(),
        });
    }

    messages
}

/// Build canonical ChatMessage list from a Responses API response body.
/// Extracts assistant messages, function calls, and reasoning from output items.
pub fn build_responses_canonical_messages(
    request_msgs: &[protocol::chat::ChatMessage],
    response_body: &serde_json::Value,
) -> Vec<protocol::chat::ChatMessage> {
    let mut msgs = request_msgs.to_vec();

    if let Some(output) = response_body.get("output").and_then(|o| o.as_array()) {
        let mut pending_reasoning: Option<String> = None;

        for item in output {
            match item.get("type").and_then(|t| t.as_str()) {
                Some("reasoning") => {
                    let text = item
                        .get("summary")
                        .and_then(|s| s.as_array())
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_default();
                    if !text.is_empty() {
                        pending_reasoning = Some(text);
                    }
                }
                Some("message") => {
                    let role = item
                        .get("role")
                        .and_then(|r| r.as_str())
                        .unwrap_or("assistant");
                    let text = item
                        .get("content")
                        .and_then(|c| c.as_array())
                        .map(|parts| {
                            parts
                                .iter()
                                .filter_map(|p| {
                                    p.get("text").and_then(|t| t.as_str()).map(String::from)
                                })
                                .collect::<Vec<_>>()
                                .join("")
                        })
                        .unwrap_or_default();

                    if role == "assistant" {
                        msgs.push(protocol::chat::ChatMessage::Assistant {
                            content: if text.is_empty() {
                                None
                            } else {
                                Some(protocol::chat::ChatContent::Text(text))
                            },
                            name: None,
                            tool_calls: None,
                            reasoning_content: pending_reasoning.take(),
                        });
                    }
                }
                Some("function_call") => {
                    let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                    let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let arguments = item.get("arguments").and_then(|a| a.as_str()).unwrap_or("");
                    msgs.push(protocol::chat::ChatMessage::Assistant {
                        content: None,
                        name: None,
                        tool_calls: Some(vec![protocol::chat::ChatToolCall {
                            id: call_id.to_string(),
                            call_type: "function".to_string(),
                            function: protocol::chat::ChatFunctionCall {
                                name: Some(name.to_string()),
                                arguments: arguments.to_string(),
                            },
                        }]),
                        reasoning_content: pending_reasoning.take(),
                    });
                }
                _ => {}
            }
        }
    }

    msgs
}
