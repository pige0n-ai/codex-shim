use protocol::chat::{ChatCompletionRequest, ChatMessage};
use protocol::provider_caps::{ProviderCapabilities, ReasoningPolicy};
use protocol::responses::ResponsesCreateRequest;

use crate::{ExtraBody, ProviderProfile};

pub struct GenericProvider {
    kind: String,
    caps: ProviderCapabilities,
    extra_body: ExtraBody,
}

impl GenericProvider {
    pub fn new(caps: ProviderCapabilities) -> Self {
        Self {
            kind: "generic-openai-chat".into(),
            caps,
            extra_body: ExtraBody::default(),
        }
    }

    pub fn named(kind: &str, caps: ProviderCapabilities) -> Self {
        Self {
            kind: kind.to_string(),
            caps,
            extra_body: ExtraBody::default(),
        }
    }

    pub fn with_extra_body(mut self, eb: ExtraBody) -> Self {
        self.extra_body = eb;
        self
    }
}

impl ProviderProfile for GenericProvider {
    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }
    fn kind(&self) -> &str {
        &self.kind
    }
    fn chat_path(&self) -> &str {
        "/v1/chat/completions"
    }
    fn extra_body(&self) -> &ExtraBody {
        &self.extra_body
    }

    fn map_reasoning(&self, req: &mut ChatCompletionRequest, source: &ResponsesCreateRequest) {
        match self.caps.reasoning_policy {
            ReasoningPolicy::None => {}
            ReasoningPolicy::SGLangReasoningContent => {
                if source.reasoning.is_some() {
                    req.extra_body["chat_template_kwargs"] = serde_json::json!({
                        "thinking": true
                    });
                    if let Some(ref reasoning) = source.reasoning {
                        req.reasoning_effort = reasoning.effort.clone();
                    }
                }
            }
            ReasoningPolicy::QwenEnableThinking => {
                if source.reasoning.is_some() {
                    req.extra_body["enable_thinking"] = serde_json::Value::Bool(true);
                    if let Some(ref reasoning) = source.reasoning {
                        req.reasoning_effort = reasoning.effort.clone();
                    }
                }
            }
            _ => {
                // OpenAI-compatible: pass through reasoning_effort only
                if let Some(ref reasoning) = source.reasoning {
                    req.reasoning_effort = reasoning.effort.clone();
                }
            }
        }
    }

    fn pre_send(&self, req: &mut ChatCompletionRequest) {
        if req.stream == Some(true) && self.caps.request_stream_usage {
            req.extra_body["stream_options"] = serde_json::json!({
                "include_usage": true
            });
        }
    }

    fn parse_reasoning_content(&self, msg: &ChatMessage) -> Option<String> {
        match msg {
            ChatMessage::Assistant {
                reasoning_content, ..
            } => reasoning_content.clone(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::provider_caps::ProviderCapabilities;

    #[test]
    fn pre_send_requests_stream_usage_when_supported() {
        let provider = GenericProvider::new(ProviderCapabilities {
            request_stream_usage: true,
            ..ProviderCapabilities::generic_chat()
        });
        let mut req = ChatCompletionRequest {
            model: "test".into(),
            messages: vec![],
            stream: Some(true),
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
        };

        provider.pre_send(&mut req);

        assert_eq!(
            req.extra_body["stream_options"]["include_usage"],
            serde_json::Value::Bool(true)
        );
    }

    #[test]
    fn pre_send_leaves_stream_options_untouched_when_usage_not_requested() {
        let provider = GenericProvider::new(ProviderCapabilities {
            request_stream_usage: false,
            ..ProviderCapabilities::generic_chat()
        });
        let mut req = ChatCompletionRequest {
            model: "test".into(),
            messages: vec![],
            stream: Some(true),
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
        };

        provider.pre_send(&mut req);

        assert!(req.extra_body.get("stream_options").is_none());
    }
}
