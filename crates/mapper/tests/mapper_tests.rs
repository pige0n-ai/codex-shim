#[cfg(test)]
mod tests {
    use mapper::*;
    use protocol::chat::*;
    use protocol::responses::*;

    fn default_config() -> MappingConfig {
        MappingConfig::default()
    }

    fn custom_format() -> CustomToolFormat {
        CustomToolFormat {
            format_type: "grammar".into(),
            syntax: "lark".into(),
            definition: "start: \"patch\"".into(),
        }
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
    fn custom_tool_mapping_uses_single_input_schema_and_preserves_format_guidance() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("edit".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool::Custom {
                name: "apply_patch".into(),
                description: "Use apply_patch".into(),
                format: custom_format(),
            }]),
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        };

        let result = responses_to_chat(&req, &[], &default_config()).unwrap();
        let tool = &result.chat_request.tools.unwrap()[0];

        assert_eq!(tool.tool_type, "function");
        assert_eq!(tool.function.name, "apply_patch");
        assert_eq!(
            tool.function.parameters.as_ref().unwrap()["required"][0],
            "input"
        );
        assert!(tool.function.parameters.as_ref().unwrap()["properties"]["input"].is_object());
        assert!(
            tool.function
                .description
                .as_deref()
                .unwrap()
                .contains("syntax: lark")
        );
    }

    #[test]
    fn apply_patch_structured_mapping_uses_ast_schema() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("edit".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool::Custom {
                name: "apply_patch".into(),
                description: "Use apply_patch".into(),
                format: custom_format(),
            }]),
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        };
        let config = MappingConfig {
            apply_patch_upstream_tool_type: apply_patch_tool::APPLY_PATCH_UPSTREAM_STRUCTURED
                .into(),
            ..MappingConfig::default()
        };

        let result = responses_to_chat(&req, &[], &config).unwrap();
        let tool = &result.chat_request.tools.unwrap()[0];

        assert_eq!(tool.function.name, "apply_patch");
        assert!(
            tool.function
                .description
                .as_deref()
                .unwrap()
                .contains("You must always include a non-empty `raw_patch` field")
        );
        let parameters = tool.function.parameters.as_ref().unwrap();
        assert!(parameters["properties"]["hunks"].is_object());
        assert!(parameters["properties"]["raw_patch"]["minLength"].as_u64() == Some(1));
        assert_eq!(parameters["required"], serde_json::json!(["raw_patch"]));
        assert!(parameters["$defs"]["update_hunk"].is_object());
        assert_eq!(tool.function.strict, Some(false));
    }

    #[test]
    fn apply_patch_structured_mapping_can_enable_strict() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("edit".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool::Custom {
                name: "apply_patch".into(),
                description: "Use apply_patch".into(),
                format: custom_format(),
            }]),
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        };
        let config = MappingConfig {
            apply_patch_upstream_tool_type: apply_patch_tool::APPLY_PATCH_UPSTREAM_STRUCTURED
                .into(),
            apply_patch_upstream_strict: true,
            ..MappingConfig::default()
        };

        let result = responses_to_chat(&req, &[], &config).unwrap();
        let tool = &result.chat_request.tools.unwrap()[0];
        assert_eq!(tool.function.strict, Some(true));
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
    fn function_call_output_image_becomes_tool_ack_and_user_image_message() {
        let req = ResponsesCreateRequest {
            model: "mimo-v2.5-pro".into(),
            input: ResponseInput::Items(vec![InputItem::FunctionCallOutput {
                id: None,
                call_id: "call_img".into(),
                output: serde_json::json!([
                    {"type": "input_text", "text": "screenshot captured"},
                    {"type": "input_image", "image_url": {"url": "data:image/png;base64,AAAA", "detail": "high"}}
                ]),
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
        assert_eq!(result.chat_request.messages.len(), 2);
        match &result.chat_request.messages[0] {
            ChatMessage::Tool {
                content,
                tool_call_id,
            } => {
                assert_eq!(tool_call_id, "call_img");
                assert_eq!(
                    content,
                    &ChatContent::Text(
                        "screenshot captured\n[Tool call call_img returned 1 image output(s) attached as following user message(s).]"
                            .into()
                    )
                );
            }
            _ => panic!("expected text tool acknowledgement"),
        }
        match &result.chat_request.messages[1] {
            ChatMessage::User {
                content: ChatContent::Parts(parts),
                ..
            } => {
                assert!(matches!(parts[0], ChatContentPart::Text { .. }));
                assert!(matches!(
                    &parts[1],
                    ChatContentPart::ImageUrl { image_url }
                        if image_url.url == "data:image/png;base64,AAAA"
                            && image_url.detail.as_deref() == Some("high")
                ));
            }
            _ => panic!("expected synthetic user image message"),
        }
    }

    #[test]
    fn custom_tool_call_history_uses_input_adapter() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Items(vec![
                InputItem::CustomToolCall {
                    id: None,
                    call_id: "call_patch".into(),
                    name: "apply_patch".into(),
                    input: "*** Begin Patch\n*** End Patch".into(),
                },
                InputItem::CustomToolCallOutput {
                    id: None,
                    call_id: "call_patch".into(),
                    output: "ok".into(),
                },
            ]),
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
        match &result.chat_request.messages[0] {
            ChatMessage::Assistant {
                tool_calls: Some(tool_calls),
                ..
            } => {
                assert_eq!(tool_calls[0].function.name.as_deref(), Some("apply_patch"));
                assert_eq!(
                    tool_calls[0].function.arguments,
                    r#"{"input":"*** Begin Patch\n*** End Patch"}"#
                );
            }
            other => panic!("expected assistant custom tool call, got {other:?}"),
        }
        assert!(matches!(
            &result.chat_request.messages[1],
            ChatMessage::Tool { tool_call_id, .. } if tool_call_id == "call_patch"
        ));
    }

    #[test]
    fn apply_patch_structured_history_uses_ast_arguments() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Items(vec![InputItem::CustomToolCall {
                id: Some("fc_patch".into()),
                call_id: "call_patch".into(),
                name: "apply_patch".into(),
                input: "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** End Patch"
                    .into(),
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
        let config = MappingConfig {
            apply_patch_upstream_tool_type: apply_patch_tool::APPLY_PATCH_UPSTREAM_STRUCTURED
                .into(),
            ..MappingConfig::default()
        };

        let result = responses_to_chat(&req, &[], &config).unwrap();
        match &result.chat_request.messages[0] {
            ChatMessage::Assistant {
                tool_calls: Some(tool_calls),
                ..
            } => {
                let args: serde_json::Value =
                    serde_json::from_str(&tool_calls[0].function.arguments).unwrap();
                assert_eq!(args["hunks"][0]["kind"], "update");
                assert_eq!(args["hunks"][0]["changes"][0]["lines"][0]["op"], "remove");
            }
            other => panic!("expected assistant custom tool call, got {other:?}"),
        }
    }

    #[test]
    fn chat_custom_function_call_maps_back_to_custom_tool_call() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("edit".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool::Custom {
                name: "apply_patch".into(),
                description: "Use apply_patch".into(),
                format: custom_format(),
            }]),
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        };
        let chat = ChatCompletionResponse {
            id: "chat_1".into(),
            object: "chat.completion".into(),
            created: 1,
            model: "deepseek-v4-pro".into(),
            choices: Some(vec![ChatCompletionChoice {
                index: 0,
                message: ChatCompletionMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    tool_calls: Some(vec![ChatToolCall {
                        id: "call_patch".into(),
                        call_type: "function".into(),
                        function: ChatFunctionCall {
                            name: Some("apply_patch".into()),
                            arguments: r#"{"input":"*** Begin Patch\n*** End Patch"}"#.into(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".into()),
            }]),
            usage: None,
            system_fingerprint: None,
        };

        let tool_context = chat_tool_context::ChatToolContext::from_response_tools(
            req.tools.as_deref().unwrap_or(&[]),
        );
        let response = response_mapper::map_chat_response_to_responses(
            &chat,
            "resp_1",
            "out_1",
            &tool_context,
            &req,
            &default_config(),
        )
        .unwrap();

        assert!(matches!(
            &response.output[0],
            ResponseOutputItem::CustomToolCall { name, input, .. }
                if name == "apply_patch" && input == "*** Begin Patch\n*** End Patch"
        ));
    }

    #[test]
    fn chat_structured_apply_patch_maps_back_to_custom_tool_call() {
        let req = ResponsesCreateRequest {
            model: "deepseek-v4-pro".into(),
            input: ResponseInput::Text("edit".into()),
            instructions: None,
            previous_response_id: None,
            store: None,
            stream: Some(false),
            max_output_tokens: None,
            temperature: None,
            top_p: None,
            tools: Some(vec![ResponseTool::Custom {
                name: "apply_patch".into(),
                description: "Use apply_patch".into(),
                format: custom_format(),
            }]),
            tool_choice: None,
            parallel_tool_calls: None,
            reasoning: None,
            text: None,
            include: None,
            metadata: None,
        };
        let chat = ChatCompletionResponse {
            id: "chat_1".into(),
            object: "chat.completion".into(),
            created: 1,
            model: "deepseek-v4-pro".into(),
            choices: Some(vec![ChatCompletionChoice {
                index: 0,
                message: ChatCompletionMessage {
                    role: "assistant".into(),
                    content: None,
                    reasoning_content: None,
                    tool_calls: Some(vec![ChatToolCall {
                        id: "call_patch".into(),
                        call_type: "function".into(),
                        function: ChatFunctionCall {
                            name: Some("apply_patch".into()),
                            arguments: serde_json::json!({
                                "hunks": [{
                                    "kind": "update",
                                    "path": "a.txt",
                                    "changes": [{
                                        "anchor": null,
                                        "lines": [
                                            {"op": "remove", "text": "old"},
                                            {"op": "add", "text": "new"}
                                        ],
                                        "end_of_file": false
                                    }]
                                }]
                            })
                            .to_string(),
                        },
                    }]),
                },
                finish_reason: Some("tool_calls".into()),
            }]),
            usage: None,
            system_fingerprint: None,
        };

        let tool_context = chat_tool_context::ChatToolContext::from_response_tools(
            req.tools.as_deref().unwrap_or(&[]),
        );
        let response = response_mapper::map_chat_response_to_responses(
            &chat,
            "resp_1",
            "out_1",
            &tool_context,
            &req,
            &MappingConfig {
                apply_patch_upstream_tool_type: apply_patch_tool::APPLY_PATCH_UPSTREAM_STRUCTURED
                    .into(),
                ..MappingConfig::default()
            },
        )
        .unwrap();

        assert!(matches!(
            &response.output[0],
            ResponseOutputItem::CustomToolCall { name, input, .. }
                if name == "apply_patch"
                    && input == "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** End Patch"
        ));
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
