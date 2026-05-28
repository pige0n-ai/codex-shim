use protocol::chat::{ChatContent, ChatMessage};
use protocol::error::ApiError;
use protocol::responses::{
    InputItem, InputMessageRole, MessageContent, ResponseInput, SummaryPart,
};

use crate::MappingConfig;
use crate::custom_tools::{custom_tool_arguments, custom_tool_arguments_for_upstream};

/// Check if the input contains any reasoning items.
pub fn has_reasoning_item(input: &ResponseInput) -> bool {
    match input {
        ResponseInput::Items(items) => items
            .iter()
            .any(|i| matches!(i, InputItem::Reasoning { .. })),
        ResponseInput::Value(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| serde_json::from_value::<InputItem>(v.clone()).ok())
            .any(|i| matches!(i, InputItem::Reasoning { .. })),
        _ => false,
    }
}

/// Convert Responses `input` to a flat `Vec<ChatMessage>`.
pub fn map_input_to_messages(input: &ResponseInput) -> Result<Vec<ChatMessage>, ApiError> {
    match input {
        ResponseInput::Text(text) => Ok(vec![ChatMessage::User {
            content: ChatContent::Text(text.clone()),
            name: None,
        }]),
        ResponseInput::Value(val) => match val {
            // Single object — it must deserialize as a valid Responses item.
            serde_json::Value::Object(_) => {
                let item = serde_json::from_value::<InputItem>(val.clone()).map_err(|e| {
                    ApiError::invalid_json(format!(
                        "input object is not a valid Responses item: {e}"
                    ))
                })?;
                map_input_item_to_messages(&item)
            }
            // Arrays must contain only valid Responses items.
            serde_json::Value::Array(arr) => {
                let mut messages = Vec::with_capacity(arr.len());
                for elem in arr {
                    let item = serde_json::from_value::<InputItem>(elem.clone()).map_err(|e| {
                        ApiError::invalid_json(format!(
                            "input array contains an invalid Responses item: {e}"
                        ))
                    })?;
                    messages.extend(map_input_item_to_messages(&item)?);
                }
                Ok(messages)
            }
            _ => Err(ApiError::invalid_json(
                "input must be a string, a Responses item object, or an array of Responses items",
            )),
        },
        ResponseInput::Items(items) => {
            let mut messages = Vec::with_capacity(items.len());
            let mut pending_reasoning: Option<String> = None;

            for item in items {
                // Handle Reasoning items: extract text for attachment to next assistant message
                if let InputItem::Reasoning {
                    content, summary, ..
                } = item
                {
                    let text = extract_reasoning_text(content.clone(), summary.clone());
                    if !text.is_empty() {
                        // Accumulate across multiple reasoning items
                        pending_reasoning
                            .get_or_insert_with(String::new)
                            .push_str(&text);
                    }
                    continue; // Reasoning items don't produce a standalone message
                }

                let mut mapped_messages = map_input_item_to_messages(item)?;

                // Attach pending reasoning_content to the first assistant message.
                if let Some(rc) = pending_reasoning.take() {
                    let mut consumed = false;
                    for msg in &mut mapped_messages {
                        if let ChatMessage::Assistant {
                            reasoning_content, ..
                        } = msg
                        {
                            *reasoning_content = Some(rc.clone());
                            consumed = true;
                            break;
                        }
                    }
                    if !consumed {
                        pending_reasoning = Some(rc);
                    }
                }

                messages.extend(mapped_messages);
            }
            if pending_reasoning.is_some() {
                return Err(ApiError::invalid_json(
                    "reasoning items must be followed by an assistant or function_call item",
                ));
            }
            Ok(messages)
        }
    }
}

pub fn apply_chat_history_mapping_overrides(
    messages: &mut [ChatMessage],
    config: &MappingConfig,
) -> Result<(), ApiError> {
    for message in messages {
        if let ChatMessage::Assistant {
            tool_calls: Some(tool_calls),
            ..
        } = message
        {
            for tool_call in tool_calls {
                let Some(name) = tool_call.function.name.as_deref() else {
                    continue;
                };
                if name == crate::apply_patch_tool::APPLY_PATCH_TOOL_NAME
                    && config.apply_patch_upstream_tool_type
                        == crate::apply_patch_tool::APPLY_PATCH_UPSTREAM_STRUCTURED
                {
                    let value: serde_json::Value =
                        serde_json::from_str(&tool_call.function.arguments).map_err(|error| {
                            ApiError::upstream_error(format!(
                                "failed to parse apply_patch history arguments: {error}"
                            ))
                        })?;
                    if let Some(input) = value.get("input").and_then(serde_json::Value::as_str) {
                        tool_call.function.arguments =
                            custom_tool_arguments_for_upstream(name, input, config);
                    }
                }
            }
        }
    }
    Ok(())
}

fn map_input_item_to_messages(item: &InputItem) -> Result<Vec<ChatMessage>, ApiError> {
    match item {
        InputItem::FunctionCallOutput {
            call_id, output, ..
        } => Ok(function_call_output_messages(call_id, output)),
        _ => map_input_item_to_message(item).map(|message| vec![message]),
    }
}

/// Convert a function_call_output value (string or array) to plain text.
fn value_to_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(items) => {
            let texts: Vec<String> = items
                .iter()
                .filter_map(|item| {
                    item.get("type")
                        .and_then(|t| t.as_str())
                        .filter(|t| *t == "input_text")
                        .and_then(|_| item.get("text"))
                        .and_then(|t| t.as_str())
                        .map(String::from)
                })
                .collect();
            if texts.is_empty() {
                val.to_string()
            } else {
                texts.join("\n")
            }
        }
        _ => val.to_string(),
    }
}

fn function_call_output_messages(call_id: &str, output: &serde_json::Value) -> Vec<ChatMessage> {
    let mut messages = vec![ChatMessage::Tool {
        content: ChatContent::Text(function_call_output_tool_text(call_id, output)),
        tool_call_id: call_id.to_string(),
    }];

    for (index, image_url) in function_call_output_images(output).into_iter().enumerate() {
        messages.push(ChatMessage::User {
            content: ChatContent::Parts(vec![
                protocol::chat::ChatContentPart::Text {
                    text: format!("Image output {} from tool call {}.", index + 1, call_id),
                },
                protocol::chat::ChatContentPart::ImageUrl {
                    image_url: protocol::chat::ChatImageUrl {
                        url: image_url.url().to_string(),
                        detail: image_url.detail().map(String::from),
                    },
                },
            ]),
            name: None,
        });
    }

    messages
}

fn function_call_output_tool_text(call_id: &str, output: &serde_json::Value) -> String {
    let image_count = function_call_output_images(output).len();
    let text = if image_count == 0 {
        value_to_string(output)
    } else {
        function_call_output_text_parts(output).join("\n")
    };
    if image_count == 0 {
        text
    } else if text.is_empty() {
        format!("Tool call {call_id} returned {image_count} image output(s).")
    } else {
        format!(
            "{text}\n[Tool call {call_id} returned {image_count} image output(s) attached as following user message(s).]"
        )
    }
}

fn function_call_output_text_parts(output: &serde_json::Value) -> Vec<String> {
    let serde_json::Value::Array(items) = output else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            item.get("type")
                .and_then(|t| t.as_str())
                .filter(|t| *t == "input_text")
                .and_then(|_| item.get("text"))
                .and_then(|t| t.as_str())
                .map(String::from)
        })
        .collect()
}

fn function_call_output_images(output: &serde_json::Value) -> Vec<protocol::common::ImageUrl> {
    let serde_json::Value::Array(items) = output else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            serde_json::from_value::<protocol::common::ContentPart>(item.clone()).ok()
        })
        .filter_map(|part| match part {
            protocol::common::ContentPart::InputImage { image_url } => Some(image_url),
            _ => None,
        })
        .collect()
}

/// Insert an `instructions` string as the first system message.
pub fn merge_instructions(messages: &mut Vec<ChatMessage>, instructions: Option<&str>) {
    if let Some(inst) = instructions {
        if inst.is_empty() {
            return;
        }
        messages.insert(
            0,
            ChatMessage::System {
                content: ChatContent::Text(inst.to_string()),
                name: None,
            },
        );
    }
}

/// Map a single `InputItem` to a `ChatMessage`.
fn map_input_item_to_message(item: &InputItem) -> Result<ChatMessage, ApiError> {
    match item {
        InputItem::Message { role, content, .. } => {
            let chat_content = message_content_to_chat(content)?;
            match role {
                InputMessageRole::User => Ok(ChatMessage::User {
                    content: chat_content,
                    name: None,
                }),
                InputMessageRole::System | InputMessageRole::Developer => Ok(ChatMessage::System {
                    content: chat_content,
                    name: None,
                }),
                InputMessageRole::Assistant => Ok(ChatMessage::Assistant {
                    content: Some(chat_content),
                    name: None,
                    tool_calls: None,
                    reasoning_content: None,
                }),
            }
        }
        InputItem::FunctionCallOutput {
            call_id, output, ..
        } => Ok(ChatMessage::Tool {
            content: ChatContent::Text(value_to_string(output)),
            tool_call_id: call_id.clone(),
        }),
        InputItem::CustomToolCallOutput {
            call_id, output, ..
        } => Ok(ChatMessage::Tool {
            content: ChatContent::Text(output.clone()),
            tool_call_id: call_id.clone(),
        }),
        InputItem::LocalShellCallOutput {
            call_id, output, ..
        } => Ok(ChatMessage::Tool {
            content: ChatContent::Text(output.clone()),
            tool_call_id: call_id.clone(),
        }),
        InputItem::ApplyPatchCallOutput {
            call_id, output, ..
        } => Ok(ChatMessage::Tool {
            content: ChatContent::Text(output.clone()),
            tool_call_id: call_id.clone(),
        }),
        // McpCall is experimental — treat as generic function_call_output
        InputItem::McpCall { .. } => Err(ApiError::not_implemented()),
        // Hosted tools — can't map
        InputItem::WebSearchCall { .. }
        | InputItem::FileSearchCall { .. }
        | InputItem::CodeInterpreterCall { .. } => {
            // These shouldn't appear as input items in practice; reject clearly
            let item_type = match item {
                InputItem::WebSearchCall { .. } => "web_search_call",
                InputItem::FileSearchCall { .. } => "file_search_call",
                InputItem::CodeInterpreterCall { .. } => "code_interpreter_call",
                _ => unreachable!(),
            };
            Err(ApiError::hosted_tool_not_supported(item_type))
        }
        // Reasoning items are opaque context — skip them (they carry no executable content)
        InputItem::Reasoning { .. } => Err(ApiError::invalid_json(
            "standalone reasoning items are not supported without an accompanying assistant message",
        )),
        InputItem::InputFile { .. } => Err(ApiError::file_input_not_supported()),
        InputItem::Unknown => Err(ApiError::unknown_input_item(None)),
        // Function call / tool call items in input are just context —
        // convert to an assistant message stub so the model sees what happened
        InputItem::FunctionCall {
            call_id,
            name,
            arguments,
            ..
        } => Ok(ChatMessage::Assistant {
            content: None,
            name: None,
            tool_calls: Some(vec![protocol::chat::ChatToolCall {
                id: call_id.clone(),
                call_type: "function".into(),
                function: protocol::chat::ChatFunctionCall {
                    name: Some(name.clone()),
                    arguments: arguments.clone(),
                },
            }]),
            reasoning_content: None,
        }),
        InputItem::CustomToolCall {
            call_id,
            name,
            input,
            ..
        } => Ok(ChatMessage::Assistant {
            content: None,
            name: None,
            tool_calls: Some(vec![protocol::chat::ChatToolCall {
                id: call_id.clone(),
                call_type: "function".into(),
                function: protocol::chat::ChatFunctionCall {
                    name: Some(name.clone()),
                    arguments: custom_tool_arguments(input),
                },
            }]),
            reasoning_content: None,
        }),
        InputItem::LocalShellCall {
            call_id,
            command,
            cwd,
            timeout_ms,
            ..
        } => {
            let args = serde_json::json!({
                "command": command,
                "cwd": cwd,
                "timeout_ms": timeout_ms,
            });
            Ok(ChatMessage::Assistant {
                content: None,
                name: None,
                tool_calls: Some(vec![protocol::chat::ChatToolCall {
                    id: call_id.clone(),
                    call_type: "function".into(),
                    function: protocol::chat::ChatFunctionCall {
                        name: Some("__codex_local_shell".into()),
                        arguments: serde_json::to_string(&args).unwrap_or_default(),
                    },
                }]),
                reasoning_content: None,
            })
        }
        InputItem::ApplyPatchCall {
            call_id,
            patch,
            diff,
            path,
            ..
        } => {
            let args = serde_json::json!({
                "patch": patch,
                "diff": diff,
                "path": path,
            });
            Ok(ChatMessage::Assistant {
                content: None,
                name: None,
                tool_calls: Some(vec![protocol::chat::ChatToolCall {
                    id: call_id.clone(),
                    call_type: "function".into(),
                    function: protocol::chat::ChatFunctionCall {
                        name: Some("__codex_apply_patch".into()),
                        arguments: serde_json::to_string(&args).unwrap_or_default(),
                    },
                }]),
                reasoning_content: None,
            })
        }
    }
}

/// Convert `MessageContent` (string or parts) to a `ChatContent`.
fn message_content_to_chat(content: &MessageContent) -> Result<ChatContent, ApiError> {
    match content {
        MessageContent::Text(t) => Ok(ChatContent::Text(t.clone())),
        MessageContent::Parts(parts) => map_parts(parts),
    }
}

fn map_parts(parts: &[protocol::common::ContentPart]) -> Result<ChatContent, ApiError> {
    if parts.len() == 1 {
        match &parts[0] {
            protocol::common::ContentPart::OutputText { text, .. }
            | protocol::common::ContentPart::InputText { text } => {
                return Ok(ChatContent::Text(text.clone()));
            }
            protocol::common::ContentPart::InputImage { image_url } => {
                return Ok(ChatContent::Parts(vec![
                    protocol::chat::ChatContentPart::ImageUrl {
                        image_url: protocol::chat::ChatImageUrl {
                            url: image_url.url().to_string(),
                            detail: image_url.detail().map(String::from),
                        },
                    },
                ]));
            }
            protocol::common::ContentPart::Refusal { .. } => {
                return Err(ApiError::unsupported_content_part("refusal"));
            }
            protocol::common::ContentPart::UnknownContentPart => {
                return Err(ApiError::unsupported_content_part("unknown"));
            }
        }
    }

    // Multiple parts → convert each one
    let mut chat_parts = Vec::with_capacity(parts.len());
    for part in parts {
        chat_parts.push(match part {
            protocol::common::ContentPart::OutputText { text, .. }
            | protocol::common::ContentPart::InputText { text } => {
                protocol::chat::ChatContentPart::Text { text: text.clone() }
            }
            protocol::common::ContentPart::InputImage { image_url } => {
                protocol::chat::ChatContentPart::ImageUrl {
                    image_url: protocol::chat::ChatImageUrl {
                        url: image_url.url().to_string(),
                        detail: image_url.detail().map(String::from),
                    },
                }
            }
            protocol::common::ContentPart::Refusal { .. } => {
                return Err(ApiError::unsupported_content_part("refusal"));
            }
            protocol::common::ContentPart::UnknownContentPart => {
                return Err(ApiError::unsupported_content_part("unknown"));
            }
        });
    }

    if chat_parts.is_empty() {
        Err(ApiError::invalid_json(
            "message content parts cannot be empty after validation",
        ))
    } else {
        Ok(ChatContent::Parts(chat_parts))
    }
}

/// Extract reasoning text from `content` (primary) or `summary` (fallback).
/// The response puts reasoning into `summary` (SummaryText), so roundtrip
/// needs to read both sources.
fn extract_reasoning_text(
    content: Option<Vec<protocol::common::ContentPart>>,
    summary: Option<Vec<SummaryPart>>,
) -> String {
    // Try content first
    if let Some(parts) = content {
        let text: String = parts
            .iter()
            .filter_map(|p| match p {
                protocol::common::ContentPart::OutputText { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return text;
        }
    }
    // Fallback to summary
    if let Some(parts) = summary {
        let text: String = parts
            .iter()
            .map(|p| match p {
                SummaryPart::SummaryText { text } => text.as_str(),
            })
            .collect::<Vec<_>>()
            .join("");
        if !text.is_empty() {
            return text;
        }
    }
    String::new()
}
