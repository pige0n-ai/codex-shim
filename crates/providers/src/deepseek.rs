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

fn map_deepseek_effort(effort: &str) -> String {
    match effort {
        "minimal" | "low" | "medium" => "high".into(),
        "high" => "high".into(),
        "xhigh" => "max".into(),
        other => other.to_string(),
    }
}
