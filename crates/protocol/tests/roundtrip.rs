#[cfg(test)]
mod tests {
    use protocol::chat::*;
    use protocol::common::{ContentPart, Usage};
    use protocol::responses::*;

    #[test]
    fn deserialize_simple_text_request() {
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": "hello world",
            "stream": false
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.model, "deepseek-v4-pro");
        assert!(matches!(req.input, ResponseInput::Text(ref t) if t == "hello world"));
        assert_eq!(req.stream, Some(false));
    }

    #[test]
    fn deserialize_input_items() {
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "output_text", "text": "hello", "annotations": []}]}
            ]
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        match req.input {
            ResponseInput::Items(items) => {
                assert_eq!(items.len(), 1);
                match &items[0] {
                    InputItem::Message { role, .. } => {
                        assert!(matches!(role, InputMessageRole::User));
                    }
                    _ => panic!("expected message item"),
                }
            }
            _ => panic!("expected items"),
        }
    }

    #[test]
    fn deserialize_output_text_without_annotations() {
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": [
                {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}]}
            ]
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        match req.input {
            ResponseInput::Items(items) => match &items[0] {
                InputItem::Message { content, .. } => match content {
                    MessageContent::Parts(parts) => match &parts[0] {
                        ContentPart::OutputText { text, annotations } => {
                            assert_eq!(text, "hello");
                            assert!(annotations.is_empty());
                        }
                        _ => panic!("expected output_text content part"),
                    },
                    _ => panic!("expected structured content"),
                },
                _ => panic!("expected message item"),
            },
            _ => panic!("expected items"),
        }
    }

    #[test]
    fn deserialize_with_tools() {
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": "weather?",
            "tools": [
                {
                    "type": "function",
                    "name": "get_weather",
                    "description": "Get weather",
                    "parameters": {"type": "object", "properties": {"location": {"type": "string"}}}
                }
            ],
            "tool_choice": "auto",
            "parallel_tool_calls": true
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.tools.unwrap().len(), 1);
        match &req.tool_choice.unwrap() {
            ToolChoice::Auto(s) => assert_eq!(s, "auto"),
            _ => panic!("expected auto"),
        }
        assert_eq!(req.parallel_tool_calls, Some(true));
    }

    #[test]
    fn deserialize_with_reasoning() {
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": "solve",
            "reasoning": {"effort": "high", "summary": "none"}
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        let r = req.reasoning.unwrap();
        assert_eq!(r.effort.unwrap(), "high");
        assert_eq!(r.summary.unwrap(), "none");
    }

    #[test]
    fn deserialize_with_structured_output() {
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": "list users",
            "text": {"format": {"type": "json_object"}}
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        let text = req.text.unwrap();
        assert!(matches!(text.format, TextFormat::JsonObject));
    }

    #[test]
    fn deserialize_with_previous_response_id() {
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": "explain",
            "previous_response_id": "resp_abc123",
            "store": true
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.previous_response_id.unwrap(), "resp_abc123");
        assert_eq!(req.store, Some(true));
    }

    #[test]
    fn round_trip_responses_object() {
        let resp = ResponsesObject {
            id: "resp_001".into(),
            object: "response".into(),
            created_at: 1710000000,
            status: "completed".into(),
            status_details: None,
            model: "deepseek-v4-pro".into(),
            output: vec![ResponseOutputItem::Message {
                id: "msg_001".into(),
                status: "completed".into(),
                role: "assistant".into(),
                content: vec![ContentPart::OutputText {
                    text: "hello".into(),
                    annotations: vec![],
                }],
            }],
            output_text: Some("hello".into()),
            usage: Some(Usage {
                input_tokens: 10,
                output_tokens: 20,
                total_tokens: 30,
                input_tokens_details: None,
                output_tokens_details: None,
            }),
            metadata: None,
            previous_response_id: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: ResponsesObject = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "resp_001");
        assert_eq!(back.output_text.unwrap(), "hello");
        assert_eq!(back.usage.unwrap().total_tokens, 30);
    }

    #[test]
    fn deserialize_chat_response_with_tool_calls() {
        let json = r#"{
            "id": "chatcmpl_123",
            "object": "chat.completion",
            "created": 1710000000,
            "model": "deepseek-v4-pro",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_123",
                                "type": "function",
                                "function": {
                                    "name": "get_weather",
                                    "arguments": "{\"location\":\"Hangzhou\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}
        }"#;
        let resp: ChatCompletionResponse = serde_json::from_str(json).unwrap();
        let choice = &resp.choices.unwrap()[0];
        let msg = &choice.message;
        assert_eq!(msg.role, "assistant");
        let tool_calls = msg.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls[0].id, "call_123");
        assert_eq!(tool_calls[0].function.name.as_ref().unwrap(), "get_weather");
    }

    #[test]
    fn deserialize_chat_response_with_reasoning() {
        let json = r#"{
            "id": "chatcmpl_456",
            "object": "chat.completion",
            "created": 1710000000,
            "model": "deepseek-v4-pro",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "answer",
                        "reasoning_content": "Let me think..."
                    },
                    "finish_reason": "stop"
                }
            ]
        }"#;
        let resp: ChatCompletionResponse = serde_json::from_str(json).unwrap();
        let msg = &resp.choices.unwrap()[0].message;
        assert_eq!(msg.reasoning_content.as_ref().unwrap(), "Let me think...");
    }

    #[test]
    fn deserialize_chat_stream_chunk() {
        let json = r#"{
            "id": "chatcmpl_789",
            "object": "chat.completion.chunk",
            "created": 1710000000,
            "model": "deepseek-v4-pro",
            "choices": [
                {
                    "index": 0,
                    "delta": {"content": "hel"},
                    "finish_reason": null
                }
            ]
        }"#;
        let chunk: ChatCompletionChunk = serde_json::from_str(json).unwrap();
        let choice = &chunk.choices.unwrap()[0];
        assert_eq!(choice.delta.content.as_ref().unwrap(), "hel");
    }

    #[test]
    fn deserialize_sse_event_response_created() {
        let json = r#"{
            "type": "response.created",
            "response": {
                "id": "resp_001",
                "object": "response",
                "created_at": 1710000000,
                "status": "in_progress",
                "model": "deepseek-v4-pro"
            }
        }"#;
        let event: protocol::sse::ResponseSseEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(
            event,
            protocol::sse::ResponseSseEvent::ResponseCreated { .. }
        ));
    }

    #[test]
    fn deserialize_codex_special_inputs() {
        // local_shell_call
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": [
                {"type": "local_shell_call", "call_id": "c1", "command": "ls -la", "cwd": "/tmp"}
            ]
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        match &req.input {
            ResponseInput::Items(items) => match &items[0] {
                InputItem::LocalShellCall {
                    command, call_id, ..
                } => {
                    assert_eq!(command, "ls -la");
                    assert_eq!(call_id, "c1");
                }
                _ => panic!("expected local_shell_call"),
            },
            _ => panic!("expected items"),
        }
    }

    #[test]
    fn deserialize_apply_patch_call() {
        let json = r#"{
            "model": "deepseek-v4-pro",
            "input": [
                {"type": "apply_patch_call", "call_id": "c2", "patch": "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new"}
            ]
        }"#;
        let req: ResponsesCreateRequest = serde_json::from_str(json).unwrap();
        match &req.input {
            ResponseInput::Items(items) => match &items[0] {
                InputItem::ApplyPatchCall { patch, call_id, .. } => {
                    assert!(patch.as_ref().unwrap().contains("@@"));
                    assert_eq!(call_id, "c2");
                }
                _ => panic!("expected apply_patch_call"),
            },
            _ => panic!("expected items"),
        }
    }

    #[test]
    fn error_constructors_work() {
        let e = protocol::error::ApiError::missing_model();
        assert_eq!(e.error.code.as_ref().unwrap(), "missing_required_parameter");

        let e = protocol::error::ApiError::unsupported_tool_type("web_search_preview", 0);
        assert!(e.error.message.contains("web_search_preview"));
        assert_eq!(e.error.param.as_ref().unwrap(), "tools[0].type");

        let e = protocol::error::ApiError::response_not_found("resp_xxx");
        assert!(e.error.message.contains("resp_xxx"));
    }
}

#[test]
fn chat_usage_alias_roundtrip() {
    use protocol::common::Usage;

    // Chat Completions naming (prompt_tokens/completion_tokens)
    let chat_json = r#"{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}"#;
    let usage: Usage = serde_json::from_str(chat_json).unwrap();
    assert_eq!(usage.input_tokens, 5);
    assert_eq!(usage.output_tokens, 3);
    assert_eq!(usage.total_tokens, 8);

    // Responses naming (input_tokens/output_tokens)
    let resp_json = r#"{"input_tokens":10,"output_tokens":7,"total_tokens":17}"#;
    let usage: Usage = serde_json::from_str(resp_json).unwrap();
    assert_eq!(usage.input_tokens, 10);
    assert_eq!(usage.output_tokens, 7);
    assert_eq!(usage.total_tokens, 17);

    // Serialize back uses Responses naming
    let out = serde_json::to_value(&usage).unwrap();
    assert_eq!(out["input_tokens"], 10);
    assert_eq!(out["output_tokens"], 7);
    assert!(out.get("prompt_tokens").is_none());
    assert!(out.get("completion_tokens").is_none());
}
