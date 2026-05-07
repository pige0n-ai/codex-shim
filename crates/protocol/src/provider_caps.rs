use serde::{Deserialize, Serialize};

/// How the adapter communicates with the upstream provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// How the adapter communicates with the upstream provider.
///
/// ⚠️ All three modes present the same **Responses API** interface to Codex.
/// `ChatCompletionsShim` is an **adapter-internal** mode; Codex itself
/// always sees `wire_api = "responses"` and never speaks Chat directly.
pub enum EndpointMode {
    /// Upstream speaks OpenAI Responses API natively (`/v1/responses`).
    /// Request/response are proxied with minimal normalization.
    NativeResponses,
    /// Upstream speaks OpenAI Responses API but does not persist state
    /// (no `previous_response_id` / `conversation` support).
    /// The adapter must materialize full conversation history on each request.
    StatelessResponses,
    /// (adapter-internal) Upstream only speaks Chat Completions.
    /// Full protocol translation: Responses ↔ Chat Completions.
    /// Codex always sees Responses API; this is purely an internal mapping.
    ChatCompletionsShim,
}

/// How the upstream delivers streaming events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamingMode {
    /// Native Responses SSE events (`response.created`, `response.output_text.delta`, etc.).
    ResponsesSse,
    /// Chat Completions SSE events (`choices[0].delta`).
    ChatCompletionsSse,
}

/// How reasoning/thinking is represented by the upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningPolicy {
    /// No reasoning support.
    None,
    /// Native OpenAI Responses `reasoning` output item.
    OpenAIResponsesReasoningItem,
    /// (Chat shim) DeepSeek-style `reasoning_content` in assistant message / delta.
    DeepSeekReasoningContent,
    /// (Chat shim) SGLang-style reasoning via `reasoning_content` (requires `chat_template_kwargs`).
    SGLangReasoningContent,
    /// (Chat shim) Qwen `enable_thinking` via `extra_body`.
    QwenEnableThinking,
    /// OpenRouter `reasoning_details` structure.
    OpenRouterReasoningDetails,
    /// (Chat shim) Generic `reasoning_content` field.
    GenericReasoningContent,
}

/// How tools are handled by the upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolPolicy {
    /// Standard `function` tools via Chat Completions or Responses API.
    FunctionToolsOnly,
    /// Native Responses API tools (includes hosted tools like web_search, code_interpreter).
    NativeResponsesTools,
    /// No tool support at all.
    NoTools,
}

/// How conversation state is managed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatePolicy {
    /// Adapter owns the state store (memory or SQLite).
    AdapterStateful,
    /// Upstream supports `previous_response_id` natively.
    UpstreamPreviousResponseId,
    /// Upstream is stateless; adapter must materialize full history.
    UpstreamStatelessFullHistory,
}

/// Complete capability declaration for an upstream provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub endpoint_mode: EndpointMode,
    pub streaming_mode: StreamingMode,
    pub reasoning_policy: ReasoningPolicy,
    pub tool_policy: ToolPolicy,
    pub state_policy: StatePolicy,

    // Fine-grained feature flags
    pub supports_function_tools: bool,
    pub supports_parallel_tool_calls: bool,
    pub supports_structured_outputs: bool,
    pub supports_json_object: bool,
    pub supports_json_schema: bool,
    pub supports_vision_input: bool,
    pub supports_hosted_web_search: bool,
    pub supports_hosted_file_search: bool,
    pub supports_code_interpreter: bool,
    pub supports_previous_response_id: bool,
    pub supports_reasoning_effort: bool,
    /// Whether chat-completions streaming requests should ask the upstream
    /// to return token usage in streamed chunks.
    pub request_stream_usage: bool,
    /// Whether streamed usage has been stable enough to use as compact/CI
    /// evidence without causing flaky assertions.
    pub reliable_stream_usage_for_compaction: bool,
    /// Whether the upstream supports the Responses WebSocket transport.
    /// codex-shim only supports HTTP/SSE; set this to false in Codex provider configs.
    #[serde(default)]
    pub supports_websockets: bool,
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::None,
            tool_policy: ToolPolicy::NoTools,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: false,
            supports_parallel_tool_calls: false,
            supports_structured_outputs: false,
            supports_json_object: false,
            supports_json_schema: false,
            supports_vision_input: false,
            supports_hosted_web_search: false,
            supports_hosted_file_search: false,
            supports_code_interpreter: false,
            supports_previous_response_id: false,
            supports_reasoning_effort: false,
            request_stream_usage: false,
            reliable_stream_usage_for_compaction: false,
            supports_websockets: false,
        }
    }
}

impl ProviderCapabilities {
    fn chat_shim(reasoning_policy: ReasoningPolicy) -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            ..Default::default()
        }
    }

    fn native_responses(reasoning_policy: ReasoningPolicy, state_policy: StatePolicy) -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::NativeResponses,
            streaming_mode: StreamingMode::ResponsesSse,
            reasoning_policy,
            tool_policy: ToolPolicy::NativeResponsesTools,
            state_policy,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_json_schema: true,
            ..Default::default()
        }
    }

    fn stateless_responses(reasoning_policy: ReasoningPolicy) -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::StatelessResponses,
            streaming_mode: StreamingMode::ResponsesSse,
            reasoning_policy,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::UpstreamStatelessFullHistory,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_json_schema: true,
            ..Default::default()
        }
    }

    /// DeepSeek Chat Completions.
    pub fn deepseek_chat() -> Self {
        Self {
            supports_json_schema: false,
            supports_reasoning_effort: true,
            request_stream_usage: true,
            reliable_stream_usage_for_compaction: true,
            ..Self::chat_shim(ReasoningPolicy::DeepSeekReasoningContent)
        }
    }

    /// SGLang Chat Completions.
    pub fn sglang_chat() -> Self {
        Self {
            supports_reasoning_effort: true,
            request_stream_usage: true,
            ..Self::chat_shim(ReasoningPolicy::SGLangReasoningContent)
        }
    }

    /// vLLM Native Responses.
    pub fn vllm_responses() -> Self {
        Self {
            supports_reasoning_effort: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::native_responses(
                ReasoningPolicy::OpenAIResponsesReasoningItem,
                StatePolicy::AdapterStateful,
            )
        }
    }

    /// vLLM Chat Completions.
    pub fn vllm_chat() -> Self {
        Self {
            supports_reasoning_effort: true,
            request_stream_usage: true,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Ollama Stateless Responses.
    pub fn ollama_responses() -> Self {
        Self {
            supports_json_schema: false,
            supports_reasoning_effort: true,
            ..Self::stateless_responses(ReasoningPolicy::OpenAIResponsesReasoningItem)
        }
    }

    /// Ollama Chat Completions.
    pub fn ollama_chat() -> Self {
        Self {
            request_stream_usage: true,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// llama.cpp Responses API backed by an internal chat conversion layer.
    pub fn llamacpp_responses() -> Self {
        Self {
            supports_json_schema: false,
            ..Self::stateless_responses(ReasoningPolicy::OpenAIResponsesReasoningItem)
        }
    }

    /// llama.cpp Chat Completions.
    pub fn llamacpp_chat() -> Self {
        Self {
            request_stream_usage: true,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// OpenRouter Stateless Responses.
    pub fn openrouter_responses() -> Self {
        Self {
            tool_policy: ToolPolicy::NativeResponsesTools,
            supports_json_schema: false,
            supports_hosted_web_search: true,
            supports_reasoning_effort: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::stateless_responses(ReasoningPolicy::OpenRouterReasoningDetails)
        }
    }

    /// OpenRouter Chat Completions.
    pub fn openrouter_chat() -> Self {
        Self {
            supports_reasoning_effort: true,
            request_stream_usage: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::chat_shim(ReasoningPolicy::OpenRouterReasoningDetails)
        }
    }

    /// Alibaba Cloud Responses (stateful).
    pub fn alibaba_responses() -> Self {
        Self {
            supports_previous_response_id: true,
            supports_reasoning_effort: true,
            supports_hosted_file_search: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::native_responses(
                ReasoningPolicy::QwenEnableThinking,
                StatePolicy::UpstreamPreviousResponseId,
            )
        }
    }

    /// Alibaba Cloud Chat Completions.
    pub fn alibaba_chat() -> Self {
        Self {
            supports_vision_input: true,
            supports_reasoning_effort: true,
            request_stream_usage: true,
            ..Self::chat_shim(ReasoningPolicy::QwenEnableThinking)
        }
    }

    /// Groq Responses API.
    pub fn groq_responses() -> Self {
        Self {
            tool_policy: ToolPolicy::NativeResponsesTools,
            supports_json_schema: false,
            supports_reasoning_effort: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::stateless_responses(ReasoningPolicy::OpenAIResponsesReasoningItem)
        }
    }

    /// Groq Chat Completions.
    pub fn groq_chat() -> Self {
        Self {
            supports_reasoning_effort: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Together AI Chat Completions.
    pub fn together_chat() -> Self {
        Self {
            reliable_stream_usage_for_compaction: false,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Fireworks Responses API.
    pub fn fireworks_responses() -> Self {
        Self {
            supports_previous_response_id: true,
            supports_reasoning_effort: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::native_responses(
                ReasoningPolicy::OpenAIResponsesReasoningItem,
                StatePolicy::UpstreamPreviousResponseId,
            )
        }
    }

    /// Fireworks AI Chat Completions.
    pub fn fireworks_chat() -> Self {
        Self {
            request_stream_usage: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// xAI Responses API.
    pub fn xai_responses() -> Self {
        Self {
            supports_previous_response_id: true,
            supports_reasoning_effort: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::native_responses(
                ReasoningPolicy::OpenAIResponsesReasoningItem,
                StatePolicy::UpstreamPreviousResponseId,
            )
        }
    }

    /// xAI Chat Completions.
    pub fn xai_chat() -> Self {
        Self {
            supports_vision_input: true,
            supports_reasoning_effort: true,
            request_stream_usage: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Amazon Bedrock Responses API.
    pub fn bedrock_responses() -> Self {
        Self {
            supports_previous_response_id: true,
            supports_reasoning_effort: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::native_responses(
                ReasoningPolicy::OpenAIResponsesReasoningItem,
                StatePolicy::UpstreamPreviousResponseId,
            )
        }
    }

    /// Amazon Bedrock Chat Completions.
    pub fn bedrock_chat() -> Self {
        Self {
            supports_reasoning_effort: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Google Gemini API (AI Studio) Chat Completions.
    pub fn gemini_chat() -> Self {
        Self {
            supports_vision_input: true,
            supports_reasoning_effort: true,
            request_stream_usage: true,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Vertex AI OpenAI-compatible Chat Completions.
    pub fn vertex_chat() -> Self {
        Self {
            supports_vision_input: true,
            supports_reasoning_effort: true,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// MiniMax OpenAI-compatible Chat Completions.
    pub fn minimax_chat() -> Self {
        Self {
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Moonshot / Kimi OpenAI-compatible Chat Completions.
    pub fn moonshot_chat() -> Self {
        Self {
            supports_vision_input: true,
            request_stream_usage: true,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Z.AI / GLM OpenAI-compatible Chat Completions.
    pub fn zai_chat() -> Self {
        Self {
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }

    /// Generic OpenAI-compatible Chat Completions backend (conservative defaults).
    pub fn generic_chat() -> Self {
        Self {
            request_stream_usage: true,
            reliable_stream_usage_for_compaction: false,
            ..Self::chat_shim(ReasoningPolicy::GenericReasoningContent)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{EndpointMode, ProviderCapabilities, StatePolicy};

    #[test]
    fn openrouter_and_fireworks_request_stream_usage_but_do_not_gate_compaction() {
        let openrouter = ProviderCapabilities::openrouter_chat();
        assert!(openrouter.request_stream_usage);
        assert!(!openrouter.reliable_stream_usage_for_compaction);

        let fireworks = ProviderCapabilities::fireworks_chat();
        assert!(fireworks.request_stream_usage);
        assert!(!fireworks.reliable_stream_usage_for_compaction);
    }

    #[test]
    fn stateful_responses_profiles_forward_previous_response_id() {
        let xai = ProviderCapabilities::xai_responses();
        assert_eq!(xai.endpoint_mode, EndpointMode::NativeResponses);
        assert_eq!(xai.state_policy, StatePolicy::UpstreamPreviousResponseId);
        assert!(xai.supports_previous_response_id);

        let bedrock = ProviderCapabilities::bedrock_responses();
        assert_eq!(bedrock.endpoint_mode, EndpointMode::NativeResponses);
        assert_eq!(
            bedrock.state_policy,
            StatePolicy::UpstreamPreviousResponseId
        );
        assert!(bedrock.supports_previous_response_id);
    }

    #[test]
    fn chat_backed_responses_profiles_are_marked_non_stateful() {
        let ollama = ProviderCapabilities::ollama_responses();
        assert_eq!(ollama.endpoint_mode, EndpointMode::StatelessResponses);
        assert_eq!(
            ollama.state_policy,
            StatePolicy::UpstreamStatelessFullHistory
        );
        assert!(!ollama.supports_previous_response_id);

        let llamacpp = ProviderCapabilities::llamacpp_responses();
        assert_eq!(llamacpp.endpoint_mode, EndpointMode::StatelessResponses);
        assert_eq!(
            llamacpp.state_policy,
            StatePolicy::UpstreamStatelessFullHistory
        );
        assert!(!llamacpp.supports_previous_response_id);
    }

    #[test]
    fn new_chat_only_profiles_stay_on_chat_completions() {
        for caps in [
            ProviderCapabilities::zai_chat(),
            ProviderCapabilities::moonshot_chat(),
            ProviderCapabilities::minimax_chat(),
            ProviderCapabilities::gemini_chat(),
            ProviderCapabilities::vertex_chat(),
        ] {
            assert_eq!(caps.endpoint_mode, EndpointMode::ChatCompletionsShim);
            assert!(!caps.supports_previous_response_id);
        }
    }
}
