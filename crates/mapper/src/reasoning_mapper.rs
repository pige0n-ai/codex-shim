use protocol::chat::{ChatCompletionRequest, ThinkingConfig};
use protocol::responses::ResponsesCreateRequest;

use crate::MappingConfig;

/// Map reasoning.effort → Chat reasoning_effort.
/// Provider-specific mappings (DeepSeek, SGLang, etc.) are handled by
/// the provider's `map_reasoning` hook. This is the generic fallback.
pub fn map_reasoning_effort(effort: Option<&str>, config: &MappingConfig) -> Option<String> {
    let effort = effort?;

    if !config.thinking_enabled {
        return Some(effort.to_string());
    }

    // Pass through unchanged; provider-specific mapping lives in ProviderProfile
    Some(effort.to_string())
}

/// Build the `thinking` config. Provider-specific defaults are handled by
/// each provider's `map_reasoning` hook.
pub fn map_thinking(config: &MappingConfig) -> Option<ThinkingConfig> {
    if config.thinking_enabled {
        Some(ThinkingConfig {
            thinking_type: "enabled".into(),
        })
    } else {
        None
    }
}

/// Apply provider-specific normalization:
/// - Drop sampling params when thinking mode is active.
///
/// DeepSeek-specific behavior (removing tools+thinking) moved to DeepSeekProvider.
pub fn normalize_sampling_params(
    chat_req: &mut ChatCompletionRequest,
    _source: &ResponsesCreateRequest,
    config: &MappingConfig,
) {
    if config.thinking_enabled
        && config.drop_sampling_params_when_thinking
        && chat_req.thinking.is_some()
    {
        chat_req.temperature = None;
        chat_req.top_p = None;
        chat_req.presence_penalty = None;
        chat_req.frequency_penalty = None;
    }
}
