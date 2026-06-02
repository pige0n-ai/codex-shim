use protocol::chat::ChatCompletionRequest;
use protocol::chat::ChatMessage;
use protocol::responses::ResponsesCreateRequest;

pub mod apply_patch_tool;
pub mod chat_tool_context;
pub mod custom_tools;
pub mod error_mapper;
pub mod input_mapper;
pub mod multimodal_mapper;
pub mod reasoning_mapper;
pub mod response_mapper;
pub mod sse_mapper;
pub mod structured_output_mapper;
pub mod tool_call_normalizer;
pub mod tool_mapper;

/// Lightweight config decoupling mapper from the providers crate.
#[derive(Debug, Clone)]
pub struct MappingConfig {
    /// If true, inject thinking params and drop sampling params.
    pub thinking_enabled: bool,
    /// Thinking effort level: "low", "medium", "high", "max"
    pub thinking_effort: Option<String>,
    /// If true, drop temperature/top_p/presence_penalty/frequency_penalty
    /// when thinking mode is active.
    pub drop_sampling_params_when_thinking: bool,
    /// If true, the upstream natively supports Responses — skip all mapping.
    pub native_responses_passthrough: bool,
    /// Provider kind string (for error messages / tracing)
    pub provider_kind: String,
    /// How to expose Codex apply_patch custom tools to upstream Chat Completions.
    pub apply_patch_upstream_tool_type: String,
    /// Whether structured apply_patch should set Chat Completions function
    /// strict mode.
    pub apply_patch_upstream_strict: bool,
}

impl Default for MappingConfig {
    fn default() -> Self {
        Self {
            thinking_enabled: false,
            thinking_effort: None,
            drop_sampling_params_when_thinking: false,
            native_responses_passthrough: false,
            provider_kind: "generic-openai-chat".into(),
            apply_patch_upstream_tool_type: apply_patch_tool::APPLY_PATCH_UPSTREAM_FREEFORM.into(),
            apply_patch_upstream_strict: false,
        }
    }
}

/// Result of mapping a Responses request to a Chat request.
#[derive(Debug)]
pub struct MappedChatRequest {
    pub chat_request: ChatCompletionRequest,
    pub tool_context: chat_tool_context::ChatToolContext,
    /// Pre-generated UUID for the response
    pub response_id: String,
    /// Pre-generated UUIDs for output items (message, function_call, etc.)
    pub output_item_ids: Vec<String>,
    /// Non-fatal warnings from the mapping process
    pub warnings: Vec<String>,
    /// True if the input contains tool-output items (function_call_output, etc.)
    /// and the adapter should attempt reasoning_content recovery from the store.
    pub needs_reasoning_recovery: bool,
}

/// Main entry point for request mapping.
///
/// `history_messages` is non-empty only when `previous_response_id` was used
/// and the store had cached messages from the prior response.
pub fn responses_to_chat(
    req: &ResponsesCreateRequest,
    history_messages: &[ChatMessage],
    config: &MappingConfig,
) -> Result<MappedChatRequest, protocol::error::ApiError> {
    if config.native_responses_passthrough {
        return Err(protocol::error::ApiError::internal(
            "native_responses_passthrough should be handled before reaching mapper",
        ));
    }

    let response_id = format!("resp_{}", uuid::Uuid::new_v4());
    let item_id = format!("msg_{}", uuid::Uuid::new_v4());
    let warnings: Vec<String> = Vec::new();

    let mut messages: Vec<ChatMessage> = Vec::new();

    // 1. Prepend history messages from previous_response_id
    messages.extend_from_slice(history_messages);

    // 2. Map instructions → system message
    input_mapper::merge_instructions(&mut messages, req.instructions.as_deref());

    // 3. Map input items → chat messages
    let input_msgs = input_mapper::map_input_to_messages(&req.input)?;
    messages.extend(input_msgs);
    input_mapper::apply_chat_history_mapping_overrides(&mut messages, config)?;

    // 3a. Enforce tool call adjacency required by OpenAI/DeepSeek API
    protocol::canonical::enforce_tool_call_adjacency(&mut messages);

    // 3b. Detect if this request needs reasoning_content recovery.
    // Codex drops reasoning items when reconstructing multi-turn input.
    // If the input has tool outputs but no reasoning items, we need to
    // recover reasoning_content from a previous stored response.
    let needs_reasoning_recovery =
        tool_mapper::has_tool_outputs(&req.input) && !input_mapper::has_reasoning_item(&req.input);

    // 4. Build ChatCompletionRequest
    let mut chat_req = ChatCompletionRequest {
        model: req.model.clone(),
        messages,
        stream: req.stream,
        max_tokens: req.max_output_tokens,
        temperature: req.temperature,
        top_p: req.top_p,
        presence_penalty: None,
        frequency_penalty: None,
        stop: None,
        tools: req
            .tools
            .as_ref()
            .map(|t| tool_mapper::map_response_tools(t, config)),
        tool_choice: req.tool_choice.as_ref().map(tool_mapper::map_tool_choice),
        parallel_tool_calls: req.parallel_tool_calls,
        response_format: structured_output_mapper::map_text_format(req.text.as_ref()),
        reasoning_effort: reasoning_mapper::map_reasoning_effort(
            req.reasoning.as_ref().and_then(|r| r.effort.as_deref()),
            config,
        ),
        thinking: reasoning_mapper::map_thinking(config),
        extra_body: serde_json::json!({}),
    };

    // 5. Provider-specific pre-send normalization
    reasoning_mapper::normalize_sampling_params(&mut chat_req, req, config);
    let tool_context = chat_tool_context::ChatToolContext::from_response_tools(
        req.tools.as_deref().unwrap_or(&[]),
    );
    tool_context.apply_to_chat_request(&mut chat_req);

    Ok(MappedChatRequest {
        chat_request: chat_req,
        tool_context,
        response_id,
        output_item_ids: vec![item_id],
        warnings,
        needs_reasoning_recovery,
    })
}
/// Map a CanonicalRequest + provider capabilities to a MappedChatRequest.
/// The caller must ensure endpoint_mode == ChatCompletionsShim before calling.
pub fn responses_to_chat_via_canonical(
    canonical: &protocol::canonical::CanonicalRequest,
    caps: &protocol::provider_caps::ProviderCapabilities,
    config: &MappingConfig,
) -> Result<MappedChatRequest, protocol::error::ApiError> {
    let response_id = format!("resp_{}", uuid::Uuid::new_v4());
    let item_id = format!("msg_{}", uuid::Uuid::new_v4());
    let warnings: Vec<String> = Vec::new();

    // Derive ChatCompletionRequest from canonical IR using real capabilities
    let mut chat_request = canonical.into_chat_request(caps);
    tool_mapper::apply_chat_tool_mapping_overrides(&mut chat_request, config);
    input_mapper::apply_chat_history_mapping_overrides(&mut chat_request.messages, config)?;
    let tool_context =
        chat_tool_context::ChatToolContext::from_response_tools(&canonical.response_tools_raw);
    tool_context.apply_to_chat_request(&mut chat_request);

    Ok(MappedChatRequest {
        chat_request,
        tool_context,
        response_id,
        output_item_ids: vec![item_id],
        warnings,
        needs_reasoning_recovery: false,
    })
}
