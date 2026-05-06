#[cfg(test)]
mod tests {
    use mapper::*;
    use protocol::chat::*;
    use protocol::responses::*;

    fn default_config() -> MappingConfig {
        MappingConfig::default()
    }

    #[test]
    fn simple_text_input() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
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
        };

        let result = responses_to_chat(&req, &[], &default_config()).unwrap();
        assert_eq!(result.chat_request.model, "deepseek-v4-pro");
        assert_eq!(result.chat_request.messages.len(), 1);
        match &result.chat_request.messages[0] {
            ChatMessage::User { content, .. } => {
                assert_eq!(content, &ChatContent::Text("hello".into()));
            }
            _ => panic!("expected user message"),
        }
    }

    #[test]
    fn instructions_becomes_system_message() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("hello".into()),
            instructions: Some("You are helpful.".into()),
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

        let result = responses_to_chat(&req, &[], &default_config()).unwrap();
        assert_eq!(result.chat_request.messages.len(), 2);
        match &result.chat_request.messages[0] {
            ChatMessage::System { content, .. } => {
                assert_eq!(content, &ChatContent::Text("You are helpful.".into()));
            }
            _ => panic!("expected system message first"),
        }
    }

    #[test]
    fn max_output_tokens_renamed() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("hello".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: Some(4096),
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

        let result = responses_to_chat(&req, &[], &default_config()).unwrap();
        assert_eq!(result.chat_request.max_tokens, Some(4096));
    }

    #[test]
    fn function_tool_mapping() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("weather?".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool::Function {
                name: "get_weather".into(),
                description: Some("Get weather".into()),
                parameters: Some(serde_json::json!({
                    "type": "object",
                    "properties": {"location": {"type": "string"}}
                })),
                strict: Some(true),
            }]),
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        };

        let result = responses_to_chat(&req, &[], &default_config()).unwrap();
        let tools = result.chat_request.tools.unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].function.name, "get_weather");
        assert_eq!(tools[0].function.strict, Some(true));
    }

    #[test]
    fn structured_output_json_object() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("list users".into()),
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
            text: Some(TextConfig {
                format: TextFormat::JsonObject,
            }),
            include: None,
            metadata: None,
        };

        let result = responses_to_chat(&req, &[], &default_config()).unwrap();
        let fmt = result.chat_request.response_format.unwrap();
        assert_eq!(fmt["type"], "json_object");
    }

    #[test]
    fn reasoning_effort_mapping() {
        // Without thinking enabled, passes through
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("think".into()),
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
            reasoning: Some(ReasoningConfig {
                effort: Some("high".into()),
                summary: None,
            }),
            text: None,
            include: None,
            metadata: None,
        };

        let result = responses_to_chat(&req, &[], &default_config()).unwrap();
        // generic config has thinking disabled, so effort passes through as-is
        assert_eq!(
            result.chat_request.reasoning_effort.as_deref(),
            Some("high")
        );
    }

    #[test]
    fn thinking_mode_drops_sampling_params() {
        let config = MappingConfig {
            thinking_enabled: true,
            thinking_effort: Some("high".into()),
            drop_sampling_params_when_thinking: true,
            ..MappingConfig::default()
        };

        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("think".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: Some(0.5),
            top_p: Some(0.9),
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: Some(ReasoningConfig {
                effort: Some("high".into()),
                summary: None,
            }),
            text: None,
            include: None,
            metadata: None,
        };

        let result = responses_to_chat(&req, &[], &config).unwrap();
        assert!(result.chat_request.thinking.is_some());
        assert_eq!(
            result.chat_request.thinking.as_ref().unwrap().thinking_type,
            "enabled"
        );
        // sampling params should be dropped
        assert_eq!(result.chat_request.temperature, None);
        assert_eq!(result.chat_request.top_p, None);
    }

    #[test]
    fn history_messages_prepended() {
        let history = vec![ChatMessage::Assistant {
            content: Some(ChatContent::Text("previous answer".into())),
            name: None,
            tool_calls: None,
            reasoning_content: None,
        }];

        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("follow-up".into()),
            instructions: None,
            previous_response_id: Some("resp_old".into()),
            store: Some(true),
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

        let result = responses_to_chat(&req, &history, &default_config()).unwrap();
        assert_eq!(result.chat_request.messages.len(), 2);
        // first message is from history
        match &result.chat_request.messages[0] {
            ChatMessage::Assistant { content, .. } => {
                assert_eq!(
                    content.as_ref().unwrap(),
                    &ChatContent::Text("previous answer".into())
                );
            }
            _ => panic!("expected assistant from history"),
        }
    }

    #[test]
    fn function_call_output_becomes_tool_message() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Items(vec![InputItem::FunctionCallOutput {
                id: None,
                call_id: "call_123".into(),
                output: "24℃".into(),
                status: None,
            }]),
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

        let result = responses_to_chat(&req, &[], &default_config()).unwrap();
        assert_eq!(result.chat_request.messages.len(), 1);
        match &result.chat_request.messages[0] {
            ChatMessage::Tool {
                content,
                tool_call_id,
            } => {
                assert_eq!(content, &ChatContent::Text("24℃".into()));
                assert_eq!(tool_call_id, "call_123");
            }
            _ => panic!("expected tool message"),
        }
    }

    #[test]
    fn value_input_invalid_object_is_rejected() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Value(serde_json::json!({
                "role": "user",
                "content": "hello"
            })),
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

        let err = responses_to_chat(&req, &[], &default_config()).unwrap_err();
        assert!(
            err.to_string()
                .contains("input object is not a valid Responses item")
        );
    }

    #[test]
    fn standalone_reasoning_item_is_rejected() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Items(vec![InputItem::Reasoning {
                id: None,
                content: Some(vec![protocol::common::ContentPart::OutputText {
                    text: "thinking".into(),
                    annotations: vec![],
                }]),
                summary: None,
                status: None,
            }]),
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

        let err = responses_to_chat(&req, &[], &default_config()).unwrap_err();
        assert!(err.to_string().contains("reasoning items must be followed"));
    }

    #[test]
    fn refusal_content_part_is_rejected() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Items(vec![InputItem::Message {
                id: None,
                role: InputMessageRole::User,
                content: MessageContent::Parts(vec![protocol::common::ContentPart::Refusal {
                    refusal: "blocked".into(),
                }]),
                status: None,
            }]),
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

        let err = responses_to_chat(&req, &[], &default_config()).unwrap_err();
        assert!(
            err.to_string()
                .contains("content part type 'refusal' is not supported")
        );
    }
}
