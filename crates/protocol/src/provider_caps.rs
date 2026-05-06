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
    pub supports_usage_in_stream_final: bool,
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
            supports_usage_in_stream_final: false,
            supports_websockets: false,
        }
    }
}

impl ProviderCapabilities {
    /// DeepSeek Chat Completions.
    pub fn deepseek_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::DeepSeekReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_json_schema: false,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// SGLang Chat Completions.
    pub fn sglang_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::SGLangReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// vLLM Native Responses.
    pub fn vllm_responses() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::NativeResponses,
            streaming_mode: StreamingMode::ResponsesSse,
            reasoning_policy: ReasoningPolicy::OpenAIResponsesReasoningItem,
            tool_policy: ToolPolicy::NativeResponsesTools,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_json_schema: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// vLLM Chat Completions.
    pub fn vllm_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::GenericReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// Ollama Stateless Responses.
    pub fn ollama_responses() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::StatelessResponses,
            streaming_mode: StreamingMode::ResponsesSse,
            reasoning_policy: ReasoningPolicy::OpenAIResponsesReasoningItem,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::UpstreamStatelessFullHistory,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// Ollama Chat Completions.
    pub fn ollama_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::GenericReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// llama.cpp Native Responses.
    pub fn llamacpp_responses() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::NativeResponses,
            streaming_mode: StreamingMode::ResponsesSse,
            reasoning_policy: ReasoningPolicy::OpenAIResponsesReasoningItem,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// llama.cpp Chat Completions.
    pub fn llamacpp_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::GenericReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// OpenRouter Stateless Responses.
    pub fn openrouter_responses() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::StatelessResponses,
            streaming_mode: StreamingMode::ResponsesSse,
            reasoning_policy: ReasoningPolicy::OpenRouterReasoningDetails,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::UpstreamStatelessFullHistory,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_hosted_web_search: true, // Opt-in: OpenRouter beta supports web search
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// OpenRouter Chat Completions.
    pub fn openrouter_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::OpenRouterReasoningDetails,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: false,
            ..Default::default()
        }
    }

    /// Alibaba Cloud Responses (stateful).
    pub fn alibaba_responses() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::NativeResponses,
            streaming_mode: StreamingMode::ResponsesSse,
            reasoning_policy: ReasoningPolicy::QwenEnableThinking,
            tool_policy: ToolPolicy::NativeResponsesTools,
            state_policy: StatePolicy::UpstreamPreviousResponseId,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_json_schema: true,
            supports_previous_response_id: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// Alibaba Cloud Chat Completions.
    pub fn alibaba_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::QwenEnableThinking,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_vision_input: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// Groq Native Responses.
    pub fn groq_responses() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::NativeResponses,
            streaming_mode: StreamingMode::ResponsesSse,
            reasoning_policy: ReasoningPolicy::GenericReasoningContent,
            tool_policy: ToolPolicy::NativeResponsesTools,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// Groq Chat Completions.
    pub fn groq_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::GenericReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_reasoning_effort: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// Together AI Chat Completions.
    pub fn together_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::GenericReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }

    /// Fireworks AI Chat Completions.
    pub fn fireworks_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::GenericReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_parallel_tool_calls: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_usage_in_stream_final: false,
            ..Default::default()
        }
    }

    /// Generic OpenAI-compatible Chat Completions backend (conservative defaults).
    pub fn generic_chat() -> Self {
        Self {
            supports_websockets: false,
            endpoint_mode: EndpointMode::ChatCompletionsShim,
            streaming_mode: StreamingMode::ChatCompletionsSse,
            reasoning_policy: ReasoningPolicy::GenericReasoningContent,
            tool_policy: ToolPolicy::FunctionToolsOnly,
            state_policy: StatePolicy::AdapterStateful,
            supports_function_tools: true,
            supports_structured_outputs: true,
            supports_json_object: true,
            supports_usage_in_stream_final: true,
            ..Default::default()
        }
    }
}
