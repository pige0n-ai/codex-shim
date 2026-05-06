use protocol::chat::{ChatCompletionRequest, ChatMessage};
use protocol::provider_caps::ProviderCapabilities;
use protocol::responses::ResponsesCreateRequest;

use crate::{ExtraBody, ProviderProfile};

pub struct SglangProvider {
    caps: ProviderCapabilities,
    extra_body: ExtraBody,
}

impl SglangProvider {
    pub fn new(caps: ProviderCapabilities) -> Self {
        Self {
            caps,
            extra_body: ExtraBody {
                chat_template_kwargs: Some(serde_json::json!({"thinking": true})),
                ..Default::default()
            },
        }
    }

    pub fn with_extra_body(mut self, eb: ExtraBody) -> Self {
        self.extra_body = eb;
        self
    }
}

impl ProviderProfile for SglangProvider {
    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }
    fn kind(&self) -> &str {
        "sglang"
    }
    fn chat_path(&self) -> &str {
        "/v1/chat/completions"
    }
    fn extra_body(&self) -> &ExtraBody {
        &self.extra_body
    }

    fn map_reasoning(&self, req: &mut ChatCompletionRequest, source: &ResponsesCreateRequest) {
        if source.reasoning.is_some() {
            req.extra_body["chat_template_kwargs"] = serde_json::json!({
                "thinking": true
            });
            if let Some(ref reasoning) = source.reasoning {
                req.reasoning_effort = reasoning.effort.clone();
            }
        }
    }

    fn pre_send(&self, _req: &mut ChatCompletionRequest) {}

    fn parse_reasoning_content(&self, msg: &ChatMessage) -> Option<String> {
        match msg {
            ChatMessage::Assistant {
                reasoning_content, ..
            } => reasoning_content.clone(),
            _ => None,
        }
    }
}
