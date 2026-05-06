use protocol::provider_caps::{
    EndpointMode, ProviderCapabilities, ReasoningPolicy, StatePolicy, ToolPolicy,
};
use providers::{ProviderProfile, create_profile};
use serde::{Deserialize, Serialize};

/// Provider profile configuration (from YAML).
///
/// Codex always connects to this adapter via `wire_api = "responses"`.
/// The profile declared here controls how the adapter talks to upstream —
/// native Responses, stateless Responses, or Chat Completions shim.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderProfileConfig {
    /// Profile name: "deepseek-chat", "vllm-responses", "sglang-chat", etc.
    #[serde(default = "default_profile_name")]
    pub profile: String,

    /// Override specific capability fields after loading the profile preset.
    #[serde(default)]
    pub capabilities: Option<ProviderCapabilitiesOverride>,

    /// Extra body fields to inject into upstream requests.
    #[serde(default)]
    pub extra_body: Option<providers::ExtraBody>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderCapabilitiesOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_function_tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_structured_outputs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_json_object: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_json_schema: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_vision_input: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_hosted_web_search: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_hosted_file_search: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_code_interpreter: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_previous_response_id: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_reasoning_effort: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supports_usage_in_stream_final: Option<bool>,
}

fn default_profile_name() -> String {
    "deepseek-chat".into()
}

impl ProviderProfileConfig {
    /// Build a ProviderProfile from this config, applying any overrides.
    pub fn build_profile(&self) -> Box<dyn ProviderProfile> {
        let override_caps = self.capabilities.as_ref().map(|o| {
            let base = match self.profile.as_str() {
                "deepseek-chat" => ProviderCapabilities::deepseek_chat(),
                "sglang-chat" => ProviderCapabilities::sglang_chat(),
                "vllm-responses" => ProviderCapabilities::vllm_responses(),
                "vllm-chat" => ProviderCapabilities::vllm_chat(),
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
                "fireworks-chat" => ProviderCapabilities::fireworks_chat(),
                "generic-chat" => ProviderCapabilities::generic_chat(),
                _ => ProviderCapabilities::generic_chat(),
            };
            apply_capability_overrides(base, o)
        });

        create_profile(&self.profile, override_caps)
    }
}

fn apply_capability_overrides(
    mut caps: ProviderCapabilities,
    overrides: &ProviderCapabilitiesOverride,
) -> ProviderCapabilities {
    if let Some(ref v) = overrides.endpoint_mode {
        caps.endpoint_mode = parse_endpoint_mode(v);
    }
    if let Some(ref v) = overrides.reasoning_policy {
        caps.reasoning_policy = parse_reasoning_policy(v);
    }
    if let Some(ref v) = overrides.tool_policy {
        caps.tool_policy = parse_tool_policy(v);
    }
    if let Some(ref v) = overrides.state_policy {
        caps.state_policy = parse_state_policy(v);
    }
    if let Some(v) = overrides.supports_function_tools {
        caps.supports_function_tools = v;
    }
    if let Some(v) = overrides.supports_parallel_tool_calls {
        caps.supports_parallel_tool_calls = v;
    }
    if let Some(v) = overrides.supports_structured_outputs {
        caps.supports_structured_outputs = v;
    }
    if let Some(v) = overrides.supports_json_object {
        caps.supports_json_object = v;
    }
    if let Some(v) = overrides.supports_json_schema {
        caps.supports_json_schema = v;
    }
    if let Some(v) = overrides.supports_vision_input {
        caps.supports_vision_input = v;
    }
    if let Some(v) = overrides.supports_hosted_web_search {
        caps.supports_hosted_web_search = v;
    }
    if let Some(v) = overrides.supports_hosted_file_search {
        caps.supports_hosted_file_search = v;
    }
    if let Some(v) = overrides.supports_code_interpreter {
        caps.supports_code_interpreter = v;
    }
    if let Some(v) = overrides.supports_previous_response_id {
        caps.supports_previous_response_id = v;
    }
    if let Some(v) = overrides.supports_reasoning_effort {
        caps.supports_reasoning_effort = v;
    }
    if let Some(v) = overrides.supports_usage_in_stream_final {
        caps.supports_usage_in_stream_final = v;
    }
    caps
}

fn parse_endpoint_mode(s: &str) -> EndpointMode {
    match s {
        "native_responses" => EndpointMode::NativeResponses,
        "stateless_responses" => EndpointMode::StatelessResponses,
        _ => EndpointMode::ChatCompletionsShim,
    }
}

fn parse_reasoning_policy(s: &str) -> ReasoningPolicy {
    match s {
        "none" => ReasoningPolicy::None,
        "openai_responses_reasoning_item" => ReasoningPolicy::OpenAIResponsesReasoningItem,
        "deepseek_reasoning_content" => ReasoningPolicy::DeepSeekReasoningContent,
        "sglang_reasoning_content" => ReasoningPolicy::SGLangReasoningContent,
        "qwen_enable_thinking" => ReasoningPolicy::QwenEnableThinking,
        "openrouter_reasoning_details" => ReasoningPolicy::OpenRouterReasoningDetails,
        _ => ReasoningPolicy::GenericReasoningContent,
    }
}

fn parse_tool_policy(s: &str) -> ToolPolicy {
    match s {
        "function_tools_only" => ToolPolicy::FunctionToolsOnly,
        "native_responses_tools" => ToolPolicy::NativeResponsesTools,
        _ => ToolPolicy::NoTools,
    }
}

fn parse_state_policy(s: &str) -> StatePolicy {
    match s {
        "adapter_stateful" => StatePolicy::AdapterStateful,
        "upstream_previous_response_id" => StatePolicy::UpstreamPreviousResponseId,
        "upstream_stateless_full_history" => StatePolicy::UpstreamStatelessFullHistory,
        _ => StatePolicy::AdapterStateful,
    }
}
