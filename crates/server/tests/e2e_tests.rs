#[cfg(test)]
mod tests {
    use codex_shim as _;

    #[test]
    fn unsupported_field_error_helper() {
        let err = protocol::error::ApiError::field_not_implemented("background");
        assert_eq!(err.error.error_type, "not_implemented");
        assert!(err.error.message.contains("background"));
        assert_eq!(err.error.param.as_deref(), Some("background"));
    }

    #[test]
    fn hosted_web_search_rejected() {
        let req = protocol::responses::ResponsesCreateRequest {
            model: "test".into(),
            input: protocol::responses::ResponseInput::Text("search".into()),
            tools: Some(vec![protocol::responses::ResponseTool::WebSearchPreview {
                user_location: None,
                search_context_size: None,
            }]),
            ..default_req()
        };
        let canonical = protocol::canonical::CanonicalRequest::from_request(&req, vec![]).unwrap();
        let result = protocol::canonical::validate_against_caps(
            &canonical,
            &protocol::provider_caps::ProviderCapabilities::generic_chat(),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("web_search"));
    }

    #[test]
    fn json_schema_rejected_when_unsupported() {
        let req = protocol::responses::ResponsesCreateRequest {
            model: "test".into(),
            input: protocol::responses::ResponseInput::Text("list".into()),
            text: Some(protocol::responses::TextConfig {
                format: protocol::responses::TextFormat::JsonSchema {
                    name: "out".into(),
                    description: None,
                    schema_: Some(serde_json::json!({})),
                    strict: None,
                },
            }),
            ..default_req()
        };
        let canonical = protocol::canonical::CanonicalRequest::from_request(&req, vec![]).unwrap();
        let caps = protocol::provider_caps::ProviderCapabilities {
            supports_json_schema: false,
            ..protocol::provider_caps::ProviderCapabilities::generic_chat()
        };
        let result = protocol::canonical::validate_against_caps(&canonical, &caps);
        assert!(result.is_err());
    }

    #[test]
    fn json_object_passes_when_supported() {
        let req = protocol::responses::ResponsesCreateRequest {
            model: "test".into(),
            input: protocol::responses::ResponseInput::Text("hi".into()),
            text: Some(protocol::responses::TextConfig {
                format: protocol::responses::TextFormat::JsonObject,
            }),
            ..default_req()
        };
        let canonical = protocol::canonical::CanonicalRequest::from_request(&req, vec![]).unwrap();
        let caps = protocol::provider_caps::ProviderCapabilities {
            supports_json_object: true,
            ..protocol::provider_caps::ProviderCapabilities::generic_chat()
        };
        let result = protocol::canonical::validate_against_caps(&canonical, &caps);
        assert!(result.is_ok());
    }

    #[test]
    fn tool_choice_specific_shape() {
        let req = protocol::canonical::CanonicalRequest {
            tool_choice: protocol::canonical::ToolChoice::Specific("my_fn".into()),
            ..default_canonical()
        };
        let chat =
            req.into_chat_request(&protocol::provider_caps::ProviderCapabilities::generic_chat());
        let tc = chat.tool_choice.unwrap();
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "my_fn");
    }

    #[test]
    fn parallel_tool_calls_preserved() {
        let req = protocol::responses::ResponsesCreateRequest {
            parallel_tool_calls: Some(false),
            ..default_req()
        };
        let canonical = protocol::canonical::CanonicalRequest::from_request(&req, vec![]).unwrap();
        assert_eq!(canonical.parallel_tool_calls, Some(false));
        let chat = canonical.into_chat_request(&protocol::provider_caps::ProviderCapabilities {
            supports_parallel_tool_calls: true,
            ..protocol::provider_caps::ProviderCapabilities::generic_chat()
        });
        assert_eq!(chat.parallel_tool_calls, Some(false));
    }

    fn default_req() -> protocol::responses::ResponsesCreateRequest {
        protocol::responses::ResponsesCreateRequest {
            model: "test".into(),
            input: protocol::responses::ResponseInput::Text("hi".into()),
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

    fn default_canonical() -> protocol::canonical::CanonicalRequest {
        protocol::canonical::CanonicalRequest::from_request(&default_req(), vec![]).unwrap()
    }
}
