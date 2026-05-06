use protocol::chat::{ChatCompletionRequest, ChatMessage};
use protocol::provider_caps::{ProviderCapabilities, StatePolicy};

use crate::{ExtraBody, ProviderProfile};

/// Provider for backends that natively speak the OpenAI Responses API.
/// Applies capability-driven field filtering and state management.
pub struct PassthroughProvider {
    caps: ProviderCapabilities,
    profile_name: String,
    extra_body: ExtraBody,
}

impl PassthroughProvider {
    pub fn new(caps: ProviderCapabilities, name: &str) -> Self {
        Self {
            caps,
            profile_name: name.to_string(),
            extra_body: ExtraBody::default(),
        }
    }

    pub fn with_extra_body(mut self, eb: ExtraBody) -> Self {
        self.extra_body = eb;
        self
    }
}

impl ProviderProfile for PassthroughProvider {
    fn capabilities(&self) -> &ProviderCapabilities {
        &self.caps
    }

    fn kind(&self) -> &str {
        &self.profile_name
    }

    fn chat_path(&self) -> &str {
        "/v1/chat/completions"
    }

    fn responses_path(&self) -> &str {
        "/v1/responses"
    }

    fn extra_body(&self) -> &ExtraBody {
        &self.extra_body
    }

    // Native Responses backends don't need Chat-specific mapping hooks.
    fn map_reasoning(
        &self,
        _req: &mut ChatCompletionRequest,
        _source: &protocol::responses::ResponsesCreateRequest,
    ) {
    }

    fn pre_send(&self, _req: &mut ChatCompletionRequest) {}

    fn parse_reasoning_content(&self, _msg: &ChatMessage) -> Option<String> {
        None
    }

    fn upstream_stateful(&self) -> bool {
        self.caps.supports_previous_response_id
            && !matches!(
                self.caps.state_policy,
                StatePolicy::UpstreamStatelessFullHistory
            )
    }

    fn requires_full_history_materialization(&self) -> bool {
        matches!(
            self.caps.state_policy,
            StatePolicy::UpstreamStatelessFullHistory
        )
    }
}
