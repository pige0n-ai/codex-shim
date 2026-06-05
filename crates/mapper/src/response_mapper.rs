use protocol::chat::{ChatCompletionResponse, ChatMessage};
use protocol::common::ContentPart;
use protocol::error::ApiError;
use protocol::responses::{
    ResponseOutputItem, ResponsesCreateRequest, ResponsesObject, SummaryPart,
};

use crate::MappingConfig;
use crate::chat_tool_context::{ChatToolContext, flatten_namespace_tool_name};
use crate::custom_tools::custom_tool_arguments;
use crate::tool_call_normalizer::normalize_chat_tool_calls;

/// Map an upstream Chat Completions response back to a Responses API object.
pub fn map_chat_response_to_responses(
    chat: &ChatCompletionResponse,
    response_id: &str,
    output_item_id: &str,
    tool_context: &ChatToolContext,
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

    let normalized_tool_calls = msg
        .tool_calls
        .as_ref()
        .map(|tool_calls| normalize_chat_tool_calls(tool_calls))
        .unwrap_or_default();

    // 2. Emit message item (if there's text content, or no tool calls)
    let text_content = extract_text_content(&msg.content);
    let output_text = if text_content.is_some() || normalized_tool_calls.is_empty() {
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

    // 3. Emit tool call items
    for tc in normalized_tool_calls {
        let fn_name = tc
            .function
            .name
            .clone()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ApiError::upstream_error("chat completion tool call missing required function.name")
            })?;
        if tc.id.is_empty() {
            return Err(ApiError::upstream_error(
                "chat completion tool call missing required id",
            ));
        }
        output.push(tool_context.output_item(
            format!("fc_{}", uuid::Uuid::new_v4()),
            "completed".into(),
            tc.id,
            fn_name,
            tc.function.arguments,
        )?);
    }

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

        let mut i = 0;
        while i < output.len() {
            let item = &output[i];
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
                    i += 1;
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
                    i += 1;
                }
                Some("function_call") => {
                    let mut tool_calls = Vec::new();
                    while let Some(item) = output.get(i)
                        && item.get("type").and_then(|t| t.as_str()) == Some("function_call")
                    {
                        let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let name = item
                            .get("namespace")
                            .and_then(|n| n.as_str())
                            .map(|namespace| flatten_namespace_tool_name(namespace, name))
                            .unwrap_or_else(|| name.to_string());
                        let arguments =
                            item.get("arguments").and_then(|a| a.as_str()).unwrap_or("");
                        tool_calls.push(protocol::chat::ChatToolCall {
                            id: call_id.to_string(),
                            call_type: "function".to_string(),
                            function: protocol::chat::ChatFunctionCall {
                                name: Some(name),
                                arguments: arguments.to_string(),
                            },
                        });
                        i += 1;
                    }
                    msgs.push(protocol::chat::ChatMessage::Assistant {
                        content: None,
                        name: None,
                        tool_calls: Some(tool_calls),
                        reasoning_content: pending_reasoning.take(),
                    });
                }
                Some("custom_tool_call") => {
                    let mut tool_calls = Vec::new();
                    while let Some(item) = output.get(i)
                        && item.get("type").and_then(|t| t.as_str()) == Some("custom_tool_call")
                    {
                        let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        let input = item.get("input").and_then(|a| a.as_str()).unwrap_or("");
                        tool_calls.push(protocol::chat::ChatToolCall {
                            id: call_id.to_string(),
                            call_type: "function".to_string(),
                            function: protocol::chat::ChatFunctionCall {
                                name: Some(name.to_string()),
                                arguments: custom_tool_arguments(input),
                            },
                        });
                        i += 1;
                    }
                    msgs.push(protocol::chat::ChatMessage::Assistant {
                        content: None,
                        name: None,
                        tool_calls: Some(tool_calls),
                        reasoning_content: pending_reasoning.take(),
                    });
                }
                Some("tool_search_call") => {
                    let mut tool_calls = Vec::new();
                    while let Some(item) = output.get(i)
                        && item.get("type").and_then(|t| t.as_str()) == Some("tool_search_call")
                    {
                        let call_id = item.get("call_id").and_then(|c| c.as_str()).unwrap_or("");
                        let arguments = item
                            .get("arguments")
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}))
                            .to_string();
                        tool_calls.push(protocol::chat::ChatToolCall {
                            id: call_id.to_string(),
                            call_type: "function".to_string(),
                            function: protocol::chat::ChatFunctionCall {
                                name: Some("tool_search".to_string()),
                                arguments,
                            },
                        });
                        i += 1;
                    }
                    msgs.push(protocol::chat::ChatMessage::Assistant {
                        content: None,
                        name: None,
                        tool_calls: Some(tool_calls),
                        reasoning_content: pending_reasoning.take(),
                    });
                }
                _ => {
                    i += 1;
                }
            }
        }
    }

    msgs
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::chat::{ChatCompletionChoice, ChatCompletionMessage};
    use protocol::responses::{CustomToolFormat, ResponseInput, ResponseTool};

    #[test]
    fn responses_canonical_messages_group_parallel_function_calls() {
        let response_body = serde_json::json!({
            "output": [
                {
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": "thinking"}]
                },
                {
                    "type": "function_call",
                    "call_id": "call_a",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"a\"}"
                },
                {
                    "type": "function_call",
                    "call_id": "call_b",
                    "name": "exec_command",
                    "arguments": "{\"cmd\":\"b\"}"
                }
            ]
        });

        let msgs = build_responses_canonical_messages(&[], &response_body);

        assert_eq!(msgs.len(), 1);
        match &msgs[0] {
            ChatMessage::Assistant {
                tool_calls: Some(tool_calls),
                reasoning_content,
                ..
            } => {
                assert_eq!(reasoning_content.as_deref(), Some("thinking"));
                assert_eq!(tool_calls.len(), 2);
                assert_eq!(tool_calls[0].id, "call_a");
                assert_eq!(tool_calls[1].id, "call_b");
            }
            other => panic!("expected grouped assistant tool calls, got {other:?}"),
        }
    }

    #[test]
    fn non_streaming_content_and_tool_calls_include_both_items() {
        let context = ChatToolContext::default();
        let response = chat_response(
            Some("checking"),
            Some(vec![chat_tool_call("call_1", "lookup", "{\"q\":\"x\"}")]),
            None,
        );

        let mapped = map_chat_response_to_responses(
            &response,
            "resp_test",
            "msg_test",
            &context,
            &default_request(None),
            &MappingConfig::default(),
        )
        .unwrap();

        assert!(matches!(
            mapped.output[0],
            ResponseOutputItem::Message { .. }
        ));
        assert!(matches!(
            mapped.output[1],
            ResponseOutputItem::FunctionCall { .. }
        ));
        assert_eq!(mapped.output_text.as_deref(), Some("checking"));
    }

    #[test]
    fn non_streaming_reasoning_precedes_message_and_tool_items() {
        let context = ChatToolContext::default();
        let response = chat_response(
            Some("checking"),
            Some(vec![chat_tool_call("call_1", "lookup", "{\"q\":\"x\"}")]),
            Some("think"),
        );

        let mapped = map_chat_response_to_responses(
            &response,
            "resp_test",
            "msg_test",
            &context,
            &default_request(None),
            &MappingConfig::default(),
        )
        .unwrap();

        assert!(matches!(
            mapped.output[0],
            ResponseOutputItem::Reasoning { .. }
        ));
        assert!(matches!(
            mapped.output[1],
            ResponseOutputItem::Message { .. }
        ));
        assert!(matches!(
            mapped.output[2],
            ResponseOutputItem::FunctionCall { .. }
        ));
    }

    #[test]
    fn non_streaming_apply_patch_preserves_invalid_arguments_as_native_input() {
        let tools = vec![ResponseTool::Custom {
            name: "apply_patch".into(),
            description: "Apply a patch".into(),
            format: CustomToolFormat {
                format_type: "grammar".into(),
                syntax: "lark".into(),
                definition: "start: /.+/".into(),
            },
        }];
        let context = ChatToolContext::from_response_tools(&tools);
        let response = chat_response(
            None,
            Some(vec![chat_tool_call(
                "call_1",
                "apply_patch",
                "{\"input\":\"patch\"}",
            )]),
            None,
        );

        let mapped = map_chat_response_to_responses(
            &response,
            "resp_test",
            "msg_test",
            &context,
            &default_request(Some(tools.clone())),
            &MappingConfig::default(),
        )
        .unwrap();
        assert!(matches!(
            &mapped.output[0],
            ResponseOutputItem::CustomToolCall { input, .. } if input == "patch"
        ));

        let bad_response = chat_response(
            None,
            Some(vec![chat_tool_call(
                "call_1",
                "apply_patch",
                "{\"input\":1}",
            )]),
            None,
        );
        let mapped = map_chat_response_to_responses(
            &bad_response,
            "resp_test",
            "msg_test",
            &context,
            &default_request(Some(tools)),
            &MappingConfig::default(),
        )
        .unwrap();
        assert!(matches!(
            &mapped.output[0],
            ResponseOutputItem::CustomToolCall { input, .. } if input == "{\"input\":1}"
        ));
    }

    #[test]
    fn non_streaming_other_custom_tool_requires_exact_string_input() {
        let tools = vec![ResponseTool::Custom {
            name: "custom_editor".into(),
            description: "Edit".into(),
            format: CustomToolFormat {
                format_type: "grammar".into(),
                syntax: "lark".into(),
                definition: "start: /.+/".into(),
            },
        }];
        let context = ChatToolContext::from_response_tools(&tools);
        let response = chat_response(
            None,
            Some(vec![chat_tool_call(
                "call_1",
                "custom_editor",
                "{\"input\":1}",
            )]),
            None,
        );

        let err = map_chat_response_to_responses(
            &response,
            "resp_test",
            "msg_test",
            &context,
            &default_request(Some(tools)),
            &MappingConfig::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("must be a string"));
    }

    #[test]
    fn non_streaming_namespace_and_tool_search_restore_through_context() {
        let tools = vec![
            ResponseTool::Namespace {
                name: "functions".into(),
                description: "Functions".into(),
                tools: vec![protocol::responses::NamespaceTool::Function {
                    name: "exec_command".into(),
                    description: Some("Run a command".into()),
                    parameters: Some(serde_json::json!({"type": "object"})),
                    strict: Some(false),
                }],
            },
            ResponseTool::ToolSearch { description: None },
        ];
        let context = ChatToolContext::from_response_tools(&tools);
        let response = chat_response(
            None,
            Some(vec![
                chat_tool_call("call_1", "functions___exec_command", "{\"cmd\":\"date\"}"),
                chat_tool_call("call_2", "tool_search", "{\"query\":\"rg\"}"),
            ]),
            None,
        );

        let mapped = map_chat_response_to_responses(
            &response,
            "resp_test",
            "msg_test",
            &context,
            &default_request(Some(tools)),
            &MappingConfig::default(),
        )
        .unwrap();

        assert!(matches!(
            &mapped.output[0],
            ResponseOutputItem::FunctionCall {
                namespace: Some(namespace),
                name,
                ..
            } if namespace == "functions" && name == "exec_command"
        ));
        assert!(matches!(
            &mapped.output[1],
            ResponseOutputItem::ToolSearchCall { arguments, .. }
                if arguments["query"] == "rg"
        ));
    }

    fn chat_response(
        content: Option<&str>,
        tool_calls: Option<Vec<protocol::chat::ChatToolCall>>,
        reasoning_content: Option<&str>,
    ) -> protocol::chat::ChatCompletionResponse {
        protocol::chat::ChatCompletionResponse {
            id: "chatcmpl_test".into(),
            object: "chat.completion".into(),
            created: 1,
            model: "test-model".into(),
            choices: Some(vec![ChatCompletionChoice {
                index: 0,
                message: ChatCompletionMessage {
                    role: "assistant".into(),
                    content: content.map(|text| protocol::chat::ChatContent::Text(text.into())),
                    tool_calls,
                    reasoning_content: reasoning_content.map(str::to_string),
                },
                finish_reason: Some("stop".into()),
            }]),
            usage: None,
            system_fingerprint: None,
        }
    }

    fn chat_tool_call(id: &str, name: &str, arguments: &str) -> protocol::chat::ChatToolCall {
        protocol::chat::ChatToolCall {
            id: id.into(),
            call_type: "function".into(),
            function: protocol::chat::ChatFunctionCall {
                name: Some(name.into()),
                arguments: arguments.into(),
            },
        }
    }

    fn default_request(tools: Option<Vec<ResponseTool>>) -> ResponsesCreateRequest {
        ResponsesCreateRequest {
            model: "test-model".into(),
            input: ResponseInput::Text("hi".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools,
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        }
    }
}
