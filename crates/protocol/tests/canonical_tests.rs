#[cfg(test)]
mod tests {
    use protocol::canonical::CanonicalRequest;
    use protocol::chat::ChatMessage;
    use protocol::provider_caps::ProviderCapabilities;
    use protocol::responses::*;

    #[test]
    fn text_input_to_chat_request() {
        let req = ResponsesCreateRequest {
            model: "test-model".into(),
            input: ResponseInput::Text("hello world".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        let caps = ProviderCapabilities::generic_chat();
        let chat = canonical.into_chat_request(&caps);

        assert_eq!(chat.model, "test-model");
        assert_eq!(chat.messages.len(), 1);
    }

    #[test]
    fn instructions_becomes_system_message() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Text("hello".into()),
            instructions: Some("You are helpful.".into()),
            ..default_req()
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        let caps = ProviderCapabilities::generic_chat();
        let chat = canonical.into_chat_request(&caps);

        assert_eq!(chat.messages.len(), 2);
        match &chat.messages[0] {
            ChatMessage::System { content, .. } => {
                assert!(
                    matches!(content, protocol::chat::ChatContent::Text(s) if s == "You are helpful.")
                );
            }
            _ => panic!("expected system message first"),
        }
    }

    #[test]
    fn json_object_text_format() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Text("list users".into()),
            text: Some(TextConfig {
                format: TextFormat::JsonObject,
            }),
            ..default_req()
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        let caps = ProviderCapabilities::generic_chat();
        let chat = canonical.into_chat_request(&caps);

        let fmt = chat.response_format.unwrap();
        assert_eq!(fmt["type"], "json_object");
    }

    #[test]
    fn function_call_output_roundtrip() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Items(vec![InputItem::FunctionCallOutput {
                id: None,
                call_id: "call_1".into(),
                output: serde_json::Value::String("result".into()),
                status: None,
            }]),
            ..default_req()
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        let caps = ProviderCapabilities::generic_chat();
        let chat = canonical.into_chat_request(&caps);

        match &chat.messages[0] {
            ChatMessage::Tool {
                content,
                tool_call_id,
            } => {
                assert_eq!(tool_call_id, "call_1");
                assert!(matches!(content, protocol::chat::ChatContent::Text(_)));
            }
            _ => panic!("expected tool message"),
        }
    }

    #[test]
    fn into_native_responses_json_basic() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Text("hello".into()),
            stream: Some(false),
            ..default_req()
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        let json = canonical.into_native_responses_json();

        assert_eq!(json["model"], "test");
        assert!(json["stream"] == false);
        assert!(json["input"].is_array());
    }

    #[test]
    fn assistant_output_text_without_annotations_is_accepted() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Value(serde_json::json!([
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "remembered"}]
                }
            ])),
            stream: Some(false),
            ..default_req()
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        assert_eq!(canonical.items.len(), 1);
    }

    #[test]
    fn parallel_tool_calls_when_cap_disabled() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Text("hello".into()),
            ..default_req()
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        let caps = ProviderCapabilities {
            supports_parallel_tool_calls: false,
            ..ProviderCapabilities::generic_chat()
        };
        let chat = canonical.into_chat_request(&caps);

        // When parallel not supported, explicitly disable
        assert_eq!(chat.parallel_tool_calls, Some(false));
    }

    #[test]
    fn parallel_tool_calls_when_cap_enabled() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Text("hello".into()),
            ..default_req()
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        let caps = ProviderCapabilities {
            supports_parallel_tool_calls: true,
            ..ProviderCapabilities::generic_chat()
        };
        let chat = canonical.into_chat_request(&caps);

        // When supported, let API default (None)
        assert_eq!(chat.parallel_tool_calls, None);
    }

    #[test]
    fn validate_rejects_web_search() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Text("search".into()),
            tools: Some(vec![ResponseTool::WebSearchPreview {
                user_location: None,
                search_context_size: None,
            }]),
            ..default_req()
        };

        let canonical = CanonicalRequest::from_request(&req, vec![]).unwrap();
        let result = protocol::canonical::validate_against_caps(
            &canonical,
            &ProviderCapabilities::generic_chat(),
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("web_search"),
            "expected web_search error, got: {err}"
        );
    }

    #[test]
    fn value_input_rejects_primitives() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Value(serde_json::json!(123)),
            ..default_req()
        };

        let err = CanonicalRequest::from_request(&req, vec![]).unwrap_err();
        assert!(err.contains("input must be a string"));
    }

    #[test]
    fn unknown_input_item_type_rejected() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Value(serde_json::json!({
                "type": "future_item",
                "payload": "hi"
            })),
            ..default_req()
        };

        let err = CanonicalRequest::from_request(&req, vec![]).unwrap_err();
        assert!(err.contains("Unknown input item type"));
    }

    #[test]
    fn refusal_content_part_rejected() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Items(vec![InputItem::Message {
                id: None,
                role: InputMessageRole::User,
                content: MessageContent::Parts(vec![protocol::common::ContentPart::Refusal {
                    refusal: "no".into(),
                }]),
                status: None,
            }]),
            ..default_req()
        };

        let err = CanonicalRequest::from_request(&req, vec![]).unwrap_err();
        assert!(err.contains("content part type 'refusal'"));
    }

    #[test]
    fn standalone_reasoning_item_rejected() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Items(vec![InputItem::Reasoning {
                id: None,
                content: Some(vec![protocol::common::ContentPart::OutputText {
                    text: "thinking".into(),
                    annotations: vec![],
                }]),
                summary: None,
                status: None,
            }]),
            ..default_req()
        };

        let err = CanonicalRequest::from_request(&req, vec![]).unwrap_err();
        assert!(err.contains("reasoning items must be followed"));
    }

    #[test]
    fn unknown_tool_rejected() {
        let req = ResponsesCreateRequest {
            model: "test".into(),
            input: ResponseInput::Text("hello".into()),
            tools: Some(vec![ResponseTool::UnknownTool]),
            ..default_req()
        };

        let err = CanonicalRequest::from_request(&req, vec![]).unwrap_err();
        assert!(err.contains("Unsupported tool type"));
    }

    fn default_req() -> ResponsesCreateRequest {
        ResponsesCreateRequest {
            model: "test-model".into(),
            input: ResponseInput::Text("hello".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        }
    }
}
