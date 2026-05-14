use protocol::chat::{ChatCompletionRequest, ChatMessage, ThinkingConfig};
use protocol::provider_caps::ProviderCapabilities;
use protocol::responses::ResponsesCreateRequest;

use crate::{ExtraBody, ProviderProfile};

pub struct DeepSeekProvider {
    caps: ProviderCapabilities,
    extra_body: ExtraBody,
}

impl DeepSeekProvider {
    pub fn new(caps: ProviderCapabilities) -> Self {
        Self {
            caps,
            extra_body: ExtraBody::default(),
        }
    }
}

impl ProviderProfile for DeepSeekProvider {
    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }
    fn kind(&self) -> &str {
        "deepseek"
    }
    fn chat_path(&self) -> &str {
        "/chat/completions"
    }
    fn extra_body(&self) -> &ExtraBody {
        &self.extra_body
    }

    fn map_reasoning(&self, req: &mut ChatCompletionRequest, source: &ResponsesCreateRequest) {
        if source.reasoning.is_some() {
            req.thinking = Some(ThinkingConfig {
                thinking_type: "enabled".into(),
            });
            if let Some(ref reasoning) = source.reasoning
                && let Some(ref effort) = reasoning.effort
            {
                req.reasoning_effort = Some(map_deepseek_effort(effort));
            }
        }
    }

    fn pre_send(&self, req: &mut ChatCompletionRequest) {
        if req.thinking.is_some() {
            req.temperature = None;
            req.top_p = None;
            req.presence_penalty = None;
            req.frequency_penalty = None;
            ensure_reasoning_content_round_trip(req);
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

fn ensure_reasoning_content_round_trip(req: &mut ChatCompletionRequest) {
    for message in &mut req.messages {
        if let ChatMessage::Assistant {
            reasoning_content, ..
        } = message
            && reasoning_content.is_none()
        {
            *reasoning_content = Some(String::new());
        }
    }
}

fn map_deepseek_effort(effort: &str) -> String {
    match effort {
        "minimal" | "low" | "medium" => "high".into(),
        "high" => "high".into(),
        "xhigh" => "max".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::chat::{
        ChatCompletionRequest, ChatContent, ChatFunctionCall, ChatToolCall, ThinkingConfig,
    };

    fn base_request(thinking: bool) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "deepseek-v4-pro".into(),
            messages: vec![ChatMessage::Assistant {
                content: Some(ChatContent::Text(String::new())),
                name: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".into(),
                    call_type: "function".into(),
                    function: ChatFunctionCall {
                        name: Some("exec_command".into()),
                        arguments: r#"{"cmd":"ls"}"#.into(),
                    },
                }]),
                reasoning_content: None,
            }],
            stream: Some(true),
            max_tokens: None,
            temperature: Some(0.7),
            top_p: Some(0.9),
            presence_penalty: Some(0.1),
            frequency_penalty: Some(0.2),
            stop: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            response_format: None,
            reasoning_effort: None,
            thinking: thinking.then_some(ThinkingConfig {
                thinking_type: "enabled".into(),
            }),
            extra_body: serde_json::json!({}),
        }
    }

    #[test]
    fn pre_send_adds_empty_reasoning_content_in_thinking_mode() {
        let provider = DeepSeekProvider::new(ProviderCapabilities::deepseek_chat());
        let mut req = base_request(true);

        provider.pre_send(&mut req);

        match &req.messages[0] {
            ChatMessage::Assistant {
                reasoning_content, ..
            } => assert_eq!(reasoning_content.as_deref(), Some("")),
            other => panic!("expected assistant message, got {other:?}"),
        }
        assert_eq!(req.temperature, None);
        assert_eq!(req.top_p, None);
        assert_eq!(req.presence_penalty, None);
        assert_eq!(req.frequency_penalty, None);
    }

    #[test]
    fn pre_send_preserves_existing_reasoning_content() {
        let provider = DeepSeekProvider::new(ProviderCapabilities::deepseek_chat());
        let mut req = base_request(true);
        if let ChatMessage::Assistant {
            reasoning_content, ..
        } = &mut req.messages[0]
        {
            *reasoning_content = Some("thinking".into());
        }

        provider.pre_send(&mut req);

        match &req.messages[0] {
            ChatMessage::Assistant {
                reasoning_content, ..
            } => assert_eq!(reasoning_content.as_deref(), Some("thinking")),
            other => panic!("expected assistant message, got {other:?}"),
        }
    }

    #[test]
    fn pre_send_leaves_reasoning_content_untouched_without_thinking_mode() {
        let provider = DeepSeekProvider::new(ProviderCapabilities::deepseek_chat());
        let mut req = base_request(false);

        provider.pre_send(&mut req);

        match &req.messages[0] {
            ChatMessage::Assistant {
                reasoning_content, ..
            } => assert_eq!(reasoning_content, &None),
            other => panic!("expected assistant message, got {other:?}"),
        }
    }
}
