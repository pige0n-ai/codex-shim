use protocol::chat::{ChatCompletionRequest, ChatMessage};
use protocol::provider_caps::{EndpointMode, ProviderCapabilities, StatePolicy};
use protocol::responses::ResponsesCreateRequest;
use serde::{Deserialize, Serialize};

mod deepseek;
mod generic;
mod passthrough;
mod sglang;
mod vllm;
mod profile_meta;

pub use deepseek::DeepSeekProvider;
pub use generic::GenericProvider;
pub use passthrough::PassthroughProvider;
pub use sglang::SglangProvider;
pub use vllm::VllmProvider;
pub use profile_meta::{ProfileCategory, ProfileMeta, all_profile_metadata, get_profile_meta, profiles_by_category};

/// Extra body fields injected into the upstream request.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtraBody {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_template_kwargs: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enable_thinking: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separate_reasoning: Option<bool>,
    /// Catch-all for provider-specific fields.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

impl ExtraBody {
    pub fn is_empty(&self) -> bool {
        self.chat_template_kwargs.is_none()
            && self.enable_thinking.is_none()
            && self.separate_reasoning.is_none()
            && self.extra.as_object().is_none_or(|o| o.is_empty())
    }

    pub fn merge_into(&self, target: &mut serde_json::Value) {
        if let Some(ref v) = self.chat_template_kwargs {
            target["chat_template_kwargs"] = v.clone();
        }
        if let Some(v) = self.enable_thinking {
            target["enable_thinking"] = serde_json::Value::Bool(v);
        }
        if let Some(v) = self.separate_reasoning {
            target["separate_reasoning"] = serde_json::Value::Bool(v);
        }
        if let Some(obj) = self.extra.as_object() {
            for (k, v) in obj {
                if !target.as_object().is_none_or(|t| t.contains_key(k)) {
                    target[k] = v.clone();
                }
            }
        }
    }
}

/// Complete provider profile: capabilities + behavior hooks.
pub trait ProviderProfile: Send + Sync {
    /// Immutable capability declaration.
    fn capabilities(&self) -> &ProviderCapabilities;

    /// The upstream chat completions path (only meaningful for ChatCompletionsShim).
    fn chat_path(&self) -> &str {
        "/v1/chat/completions"
    }

    /// The upstream responses path (meaningful for NativeResponses / StatelessResponses).
    fn responses_path(&self) -> &str {
        "/v1/responses"
    }

    /// The upstream models path.
    fn models_path(&self) -> &str {
        "/v1/models"
    }

    /// Provider kind string for diagnostics.
    fn kind(&self) -> &str;

    /// Extra body fields to merge into the upstream request.
    fn extra_body(&self) -> &ExtraBody;

    // --- Chat Shim hooks (only called when endpoint_mode == ChatCompletionsShim) ---

    /// Apply reasoning/thinking configuration to the Chat request.
    fn map_reasoning(&self, req: &mut ChatCompletionRequest, source: &ResponsesCreateRequest);

    /// Last-chance normalization before sending.
    fn pre_send(&self, req: &mut ChatCompletionRequest);

    /// Extract reasoning content from an assistant chat message.
    fn parse_reasoning_content(&self, msg: &ChatMessage) -> Option<String>;

    // --- State policy hooks ---

    /// Whether the upstream stores conversation state natively
    /// (i.e. `previous_response_id` works without adapter help).
    fn upstream_stateful(&self) -> bool {
        self.capabilities().supports_previous_response_id
    }

    /// Whether the adapter must materialize full history for stateless backends.
    fn requires_full_history_materialization(&self) -> bool {
        matches!(
            self.capabilities().state_policy,
            StatePolicy::UpstreamStatelessFullHistory
        )
    }
}

pub const CANONICAL_PROFILE_NAMES: &[&str] = &[
    "deepseek-chat",
    "sglang-chat",
    "vllm-responses",
    "vllm-chat",
    "ollama-responses",
    "ollama-chat",
    "llamacpp-responses",
    "llamacpp-chat",
    "openrouter-responses",
    "openrouter-chat",
    "alibaba-responses",
    "alibaba-chat",
    "groq-responses",
    "groq-chat",
    "together-chat",
    "fireworks-responses",
    "fireworks-chat",
    "xai-responses",
    "xai-chat",
    "bedrock-responses",
    "bedrock-chat",
    "gemini-chat",
    "vertex-chat",
    "minimax-chat",
    "moonshot-chat",
    "zai-chat",
    "generic-chat",
];

pub const SUPPORTED_PROFILE_NAMES: &[&str] = &[
    "deepseek-chat",
    "sglang-chat",
    "vllm-responses",
    "vllm-chat",
    "ollama-responses",
    "ollama-chat",
    "llamacpp-responses",
    "llamacpp-chat",
    "openrouter-responses",
    "openrouter-chat",
    "alibaba-responses",
    "alibaba-chat",
    "groq-responses",
    "groq-chat",
    "together-chat",
    "fireworks-responses",
    "fireworks-chat",
    "xai-responses",
    "xai-chat",
    "bedrock-responses",
    "bedrock-chat",
    "gemini-chat",
    "vertex-chat",
    "minimax-chat",
    "moonshot-chat",
    "zai-chat",
    "generic-chat",
    "deepseek",
    "sglang",
    "vllm",
    "generic",
    "generic-openai-chat",
];

pub fn preset_capabilities(profile_name: &str) -> Option<ProviderCapabilities> {
    let caps = match profile_name {
        "deepseek-chat" | "deepseek" => ProviderCapabilities::deepseek_chat(),
        "sglang-chat" | "sglang" => ProviderCapabilities::sglang_chat(),
        "vllm-responses" => ProviderCapabilities::vllm_responses(),
        "vllm-chat" | "vllm" => ProviderCapabilities::vllm_chat(),
        "ollama-responses" => ProviderCapabilities::ollama_responses(),
        "ollama-chat" => ProviderCapabilities::ollama_chat(),
        "llamacpp-responses" => ProviderCapabilities::llamacpp_responses(),
        "llamacpp-chat" => ProviderCapabilities::llamacpp_chat(),
        "openrouter-responses" => ProviderCapabilities::openrouter_responses(),
        "openrouter-chat" => ProviderCapabilities::openrouter_chat(),
        "alibaba-responses" => ProviderCapabilities::alibaba_responses(),
        "alibaba-chat" => ProviderCapabilities::alibaba_chat(),
        "groq-responses" => ProviderCapabilities::groq_responses(),
        "groq-chat" => ProviderCapabilities::groq_chat(),
        "together-chat" => ProviderCapabilities::together_chat(),
        "fireworks-responses" => ProviderCapabilities::fireworks_responses(),
        "fireworks-chat" => ProviderCapabilities::fireworks_chat(),
        "xai-responses" => ProviderCapabilities::xai_responses(),
        "xai-chat" => ProviderCapabilities::xai_chat(),
        "bedrock-responses" => ProviderCapabilities::bedrock_responses(),
        "bedrock-chat" => ProviderCapabilities::bedrock_chat(),
        "gemini-chat" => ProviderCapabilities::gemini_chat(),
        "vertex-chat" => ProviderCapabilities::vertex_chat(),
        "minimax-chat" => ProviderCapabilities::minimax_chat(),
        "moonshot-chat" => ProviderCapabilities::moonshot_chat(),
        "zai-chat" => ProviderCapabilities::zai_chat(),
        "generic-chat" | "generic" | "generic-openai-chat" => ProviderCapabilities::generic_chat(),
        _ => return None,
    };
    Some(caps)
}

pub fn is_supported_profile_name(profile_name: &str) -> bool {
    SUPPORTED_PROFILE_NAMES.contains(&profile_name)
}

/// Create a provider profile from capability presets.
///
/// The returned profile always presents the **Responses API** to Codex.
/// `ChatCompletionsShim` is an adapter-internal mode used when the upstream
/// does not speak native Responses.
///
/// `override_caps` allows callers to tweak individual capability flags
/// (e.g. from YAML config or CLI flags).
pub fn create_profile(
    profile_name: &str,
    override_caps: Option<ProviderCapabilities>,
) -> Box<dyn ProviderProfile> {
    let mut caps =
        preset_capabilities(profile_name).unwrap_or_else(ProviderCapabilities::generic_chat);

    if let Some(overrides) = override_caps {
        caps = overrides;
    }

    // Dispatch to the appropriate runtime implementation based on endpoint mode.
    // For ChatCompletionsShim, we use the provider-specific handler.
    // For Native/Stateless Responses, we use a common pass-through handler
    // with capability-driven field filtering.
    match caps.endpoint_mode {
        EndpointMode::ChatCompletionsShim => create_chat_shim_profile(profile_name, caps),
        EndpointMode::NativeResponses | EndpointMode::StatelessResponses => {
            create_responses_profile(profile_name, caps)
        }
    }
}

fn create_chat_shim_profile(name: &str, caps: ProviderCapabilities) -> Box<dyn ProviderProfile> {
    match name {
        "deepseek-chat" | "deepseek" => Box::new(DeepSeekProvider::new(caps)),
        "sglang-chat" | "sglang" => Box::new(SglangProvider::new(caps)),
        "vllm-chat" | "vllm" => Box::new(VllmProvider::new(caps)),
        _ => Box::new(GenericProvider::named(name, caps)),
    }
}

fn create_responses_profile(name: &str, caps: ProviderCapabilities) -> Box<dyn ProviderProfile> {
    Box::new(PassthroughProvider::new(caps, name))
}
