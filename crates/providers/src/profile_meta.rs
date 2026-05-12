use serde::{Deserialize, Serialize};

/// Category for grouping provider profiles in the interactive setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileCategory {
    HostedApi,
    LocalSelfHosted,
    Generic,
}

/// Metadata for a provider profile, used by the interactive setup wizard.
#[derive(Debug, Clone)]
pub struct ProfileMeta {
    pub name: &'static str,
    pub category: ProfileCategory,
    pub display_name: &'static str,
    pub description: &'static str,
    pub recommended_models: &'static [&'static str],
    pub default_context_window: i64,
    pub requires_api_key: bool,
}

pub const PROFILE_METADATA: &[ProfileMeta] = &[
    // ── HostedApi ────────────────────────────────────────────────────────
    ProfileMeta {
        name: "deepseek-chat",
        category: ProfileCategory::HostedApi,
        display_name: "DeepSeek",
        description: "Chat Completions with reasoning support",
        recommended_models: &["deepseek-v4-pro", "deepseek-r1"],
        default_context_window: 1_000_000,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "openrouter-chat",
        category: ProfileCategory::HostedApi,
        display_name: "OpenRouter",
        description: "Multi-model chat router",
        recommended_models: &["moonshotai/kimi-k2.6", "anthropic/claude-sonnet-4"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "openrouter-responses",
        category: ProfileCategory::HostedApi,
        display_name: "OpenRouter",
        description: "Stateless Responses (native /responses)",
        recommended_models: &["moonshotai/kimi-k2.6"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "alibaba-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Alibaba DashScope / Qwen",
        description: "Chat Completions",
        recommended_models: &["qwen3.6-plus", "qwen3.6-max"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "alibaba-responses",
        category: ProfileCategory::HostedApi,
        display_name: "Alibaba DashScope / Qwen",
        description: "Native Responses (stateful)",
        recommended_models: &["qwen3.6-plus"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "groq-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Groq",
        description: "Chat Completions (fast inference)",
        recommended_models: &["llama-3.3-70b-versatile"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "groq-responses",
        category: ProfileCategory::HostedApi,
        display_name: "Groq",
        description: "Stateless Responses",
        recommended_models: &["llama-3.3-70b-versatile"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "together-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Together AI",
        description: "Chat Completions",
        recommended_models: &["meta-llama/Llama-3.3-70B-Instruct-Turbo"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "fireworks-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Fireworks AI",
        description: "Chat Completions",
        recommended_models: &["accounts/fireworks/models/qwen3-235b-a22b"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "fireworks-responses",
        category: ProfileCategory::HostedApi,
        display_name: "Fireworks AI",
        description: "Native Responses (stateful)",
        recommended_models: &["accounts/fireworks/models/qwen3-235b-a22b"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "xai-chat",
        category: ProfileCategory::HostedApi,
        display_name: "xAI / Grok",
        description: "Chat Completions",
        recommended_models: &["grok-4.20-reasoning"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "xai-responses",
        category: ProfileCategory::HostedApi,
        display_name: "xAI / Grok",
        description: "Native Responses (stateful)",
        recommended_models: &["grok-4.20-reasoning"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "bedrock-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Amazon Bedrock",
        description: "Chat Completions",
        recommended_models: &["amazon.nova-pro-v1:0"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "bedrock-responses",
        category: ProfileCategory::HostedApi,
        display_name: "Amazon Bedrock",
        description: "Native Responses (stateful)",
        recommended_models: &["amazon.nova-pro-v1:0"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "gemini-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Google Gemini",
        description: "Chat Completions",
        recommended_models: &["gemini-3-flash-preview", "gemini-2.5-pro"],
        default_context_window: 1_048_576,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "vertex-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Google Vertex AI",
        description: "Chat Completions (OAuth auth)",
        recommended_models: &["gemini-2.5-flash"],
        default_context_window: 1_048_576,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "minimax-chat",
        category: ProfileCategory::HostedApi,
        display_name: "MiniMax",
        description: "Chat Completions",
        recommended_models: &["MiniMax-M2.7"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "moonshot-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Moonshot / Kimi",
        description: "Chat Completions",
        recommended_models: &["kimi-k2.6"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    ProfileMeta {
        name: "zai-chat",
        category: ProfileCategory::HostedApi,
        display_name: "Z.AI / GLM",
        description: "Chat Completions",
        recommended_models: &["glm-5.1"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
    // ── LocalSelfHosted ──────────────────────────────────────────────────
    ProfileMeta {
        name: "sglang-chat",
        category: ProfileCategory::LocalSelfHosted,
        display_name: "SGLang",
        description: "Chat Completions (reasoning via chat_template_kwargs)",
        recommended_models: &["local-model"],
        default_context_window: 131_072,
        requires_api_key: false,
    },
    ProfileMeta {
        name: "vllm-chat",
        category: ProfileCategory::LocalSelfHosted,
        display_name: "vLLM",
        description: "Chat Completions",
        recommended_models: &["local-model"],
        default_context_window: 131_072,
        requires_api_key: false,
    },
    ProfileMeta {
        name: "vllm-responses",
        category: ProfileCategory::LocalSelfHosted,
        display_name: "vLLM",
        description: "Native Responses",
        recommended_models: &["local-model"],
        default_context_window: 131_072,
        requires_api_key: false,
    },
    ProfileMeta {
        name: "ollama-chat",
        category: ProfileCategory::LocalSelfHosted,
        display_name: "Ollama",
        description: "Chat Completions (stable)",
        recommended_models: &["qwen3.5:32b", "llama3.1:70b"],
        default_context_window: 131_072,
        requires_api_key: false,
    },
    ProfileMeta {
        name: "ollama-responses",
        category: ProfileCategory::LocalSelfHosted,
        display_name: "Ollama",
        description: "Stateless Responses (Ollama >= 0.13)",
        recommended_models: &["qwen3.5:32b"],
        default_context_window: 131_072,
        requires_api_key: false,
    },
    ProfileMeta {
        name: "llamacpp-chat",
        category: ProfileCategory::LocalSelfHosted,
        display_name: "llama.cpp",
        description: "Chat Completions",
        recommended_models: &["local-model"],
        default_context_window: 131_072,
        requires_api_key: false,
    },
    ProfileMeta {
        name: "llamacpp-responses",
        category: ProfileCategory::LocalSelfHosted,
        display_name: "llama.cpp",
        description: "Stateless Responses",
        recommended_models: &["local-model"],
        default_context_window: 131_072,
        requires_api_key: false,
    },
    // ── Generic ──────────────────────────────────────────────────────────
    ProfileMeta {
        name: "generic-chat",
        category: ProfileCategory::Generic,
        display_name: "Generic OpenAI-compatible",
        description: "Conservative defaults for any /chat/completions backend",
        recommended_models: &["model-slug"],
        default_context_window: 131_072,
        requires_api_key: true,
    },
];

/// Look up profile metadata by canonical name.
pub fn get_profile_meta(name: &str) -> Option<&'static ProfileMeta> {
    PROFILE_METADATA.iter().find(|m| m.name == name)
}

/// Return all profiles belonging to a given category.
pub fn profiles_by_category(category: ProfileCategory) -> Vec<&'static ProfileMeta> {
    PROFILE_METADATA
        .iter()
        .filter(|m| m.category == category)
        .collect()
}

/// Return the full profile metadata array.
pub fn all_profile_metadata() -> &'static [ProfileMeta] {
    PROFILE_METADATA
}
