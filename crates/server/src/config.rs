use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::provider_profile_config::ProviderProfileConfig;
use protocol::models::CatalogModelSpec;

/// Top-level configuration for the responses-adapter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub upstream: UpstreamConfig,
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub models: ModelsConfig,
    #[serde(default)]
    pub features: Option<FeaturesConfig>,
    #[serde(default)]
    pub reasoning: ReasoningSettings,
    #[serde(default)]
    pub state: StateConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_listen")]
    pub listen: String,
    #[serde(default = "default_base_path")]
    pub base_path: String,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CorsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allow_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_auth_mode")]
    pub mode: String,
    #[serde(default)]
    pub accepted_bearer_tokens: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCommandConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_auth_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_auth_refresh")]
    pub refresh_interval_ms: u64,
    #[serde(default)]
    pub cwd: Option<String>,
}

fn default_auth_timeout() -> u64 {
    5000
}
fn default_auth_refresh() -> u64 {
    300000
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamConfig {
    #[serde(default = "default_upstream_base")]
    pub base_url: String,
    #[serde(default = "default_chat_path")]
    pub chat_path: String,
    #[serde(default = "default_responses_path")]
    pub responses_path: String,
    #[serde(default = "default_models_path")]
    pub models_path: String,
    #[serde(default = "default_upstream_key_env")]
    pub api_key_env: String,
    #[serde(default)]
    pub query_params: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub http_headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub env_http_headers: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub requires_openai_auth: bool,
    #[serde(default)]
    pub auth_command: Option<AuthCommandConfig>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout_seconds: u64,
    #[serde(default)]
    pub max_retries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider profile name: "deepseek-chat", "vllm-responses", etc.
    #[serde(default = "default_provider_kind")]
    pub kind: String,
    /// Legacy mode field — now superseded by ProviderProfileConfig.
    #[serde(default)]
    pub mode: String,
    /// Provider profile configuration (capabilities, endpoint mode, etc.).
    #[serde(default)]
    pub profile_config: Option<ProviderProfileConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    #[serde(default = "default_model")]
    pub default: String,
    #[serde(default)]
    pub map: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub catalog: Vec<CatalogModelSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeaturesConfig {
    #[serde(default = "default_true")]
    pub streaming: bool,
    #[serde(default = "default_true")]
    pub tools: bool,
    #[serde(default = "default_true")]
    pub parallel_tool_calls: bool,
    #[serde(default = "default_true")]
    pub structured_outputs: bool,
    #[serde(default = "default_true")]
    pub multimodal_images: bool,
    #[serde(default = "default_true")]
    pub previous_response_id: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningSettings {
    /// Global reasoning enable/disable. Provider-specific behavior is controlled
    /// by ProviderProfileConfig / ProviderCapabilities.
    #[serde(default)]
    pub enabled: bool,
    /// Default reasoning effort: "low", "medium", "high", "xhigh".
    #[serde(default = "default_effort")]
    pub effort: String,
    #[serde(default)]
    pub expose_reasoning_to_client: bool,
    #[serde(default = "default_true")]
    pub persist_reasoning_for_tool_calls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateConfig {
    #[serde(default = "default_state_backend")]
    pub backend: String,
    #[serde(default = "default_ttl")]
    pub ttl_seconds: u64,
    #[serde(default = "default_cleanup_interval")]
    pub cleanup_interval_seconds: u64,
    #[serde(default)]
    pub sqlite_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_true")]
    pub redact_api_keys: bool,
    #[serde(default)]
    pub redact_message_content: bool,
    #[serde(default)]
    pub log_sse_events: bool,
}

// --- Defaults ---

fn default_listen() -> String {
    "127.0.0.1:8787".into()
}
fn default_base_path() -> String {
    "/v1".into()
}
fn default_auth_mode() -> String {
    "optional-bearer".into()
}
fn default_upstream_base() -> String {
    "https://api.deepseek.com".into()
}
fn default_responses_path() -> String {
    "/responses".into()
}
fn default_chat_path() -> String {
    "/chat/completions".into()
}
fn default_models_path() -> String {
    "/models".into()
}
fn default_upstream_key_env() -> String {
    "DEEPSEEK_API_KEY".into()
}
fn default_timeout() -> u64 {
    900
}
fn default_connect_timeout() -> u64 {
    30
}
fn default_provider_kind() -> String {
    "deepseek-chat".into()
}
fn default_effort() -> String {
    "high".into()
}
fn default_model() -> String {
    "deepseek-v4-pro".into()
}
fn default_state_backend() -> String {
    "memory".into()
}
fn default_ttl() -> u64 {
    86400
}
fn default_cleanup_interval() -> u64 {
    3600
}
fn default_log_level() -> String {
    "info".into()
}
fn default_true() -> bool {
    true
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            base_path: default_base_path(),
            cors: CorsConfig::default(),
            auth: AuthConfig::default(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            mode: default_auth_mode(),
            accepted_bearer_tokens: vec![],
        }
    }
}

impl Default for UpstreamConfig {
    fn default() -> Self {
        Self {
            base_url: default_upstream_base(),
            chat_path: default_chat_path(),
            responses_path: default_responses_path(),
            models_path: default_models_path(),
            api_key_env: default_upstream_key_env(),
            query_params: std::collections::HashMap::new(),
            http_headers: std::collections::HashMap::new(),
            env_http_headers: std::collections::HashMap::new(),
            requires_openai_auth: false,
            auth_command: None,
            timeout_seconds: default_timeout(),
            connect_timeout_seconds: default_connect_timeout(),
            max_retries: 2,
        }
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: default_provider_kind(),
            mode: String::new(),
            profile_config: None,
        }
    }
}

impl Default for ModelsConfig {
    fn default() -> Self {
        Self {
            default: default_model(),
            map: std::collections::HashMap::new(),
            catalog: Vec::new(),
        }
    }
}

impl Default for FeaturesConfig {
    fn default() -> Self {
        Self {
            streaming: true,
            tools: true,
            parallel_tool_calls: true,
            structured_outputs: true,
            multimodal_images: true,
            previous_response_id: true,
        }
    }
}

impl Default for ReasoningSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            effort: default_effort(),
            expose_reasoning_to_client: false,
            persist_reasoning_for_tool_calls: true,
        }
    }
}

impl Default for StateConfig {
    fn default() -> Self {
        Self {
            backend: default_state_backend(),
            ttl_seconds: default_ttl(),
            cleanup_interval_seconds: default_cleanup_interval(),
            sqlite_path: None,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            redact_api_keys: true,
            redact_message_content: false,
            log_sse_events: false,
        }
    }
}

impl Config {
    /// Load config with layering: file → env vars → CLI overrides.
    pub fn load(config_path: Option<&str>) -> anyhow::Result<Self> {
        let mut config = if let Some(path) = config_path {
            let expanded = expand_tilde(path);
            let content = std::fs::read_to_string(&expanded)?;
            serde_yaml::from_str(&content)?
        } else {
            // Try default path
            if let Some(default_path) = default_config_path() {
                if let Ok(content) = std::fs::read_to_string(&default_path) {
                    serde_yaml::from_str(&content)?
                } else {
                    Config::default()
                }
            } else {
                Config::default()
            }
        };

        // Override with env vars: CODEX_SHIM_SERVER__LISTEN etc.
        Self::apply_env_overrides(&mut config);

        Ok(config)
    }

    fn apply_env_overrides(config: &mut Config) {
        if let Ok(v) = std::env::var("CODEX_SHIM_SERVER__LISTEN") {
            config.server.listen = v;
        }
        if let Ok(v) = std::env::var("CODEX_SHIM_UPSTREAM__BASE_URL") {
            config.upstream.base_url = v;
        }
        if let Ok(v) = std::env::var("CODEX_SHIM_UPSTREAM__API_KEY_ENV") {
            config.upstream.api_key_env = v;
        }
        if let Ok(v) = std::env::var("CODEX_SHIM_UPSTREAM__CHAT_PATH") {
            config.upstream.chat_path = v;
        }
        if let Ok(v) = std::env::var("CODEX_SHIM_PROVIDER__KIND") {
            config.provider.kind = v;
        }
        if let Ok(v) = std::env::var("CODEX_SHIM_REASONING__ENABLED") {
            config.reasoning.enabled = v.parse().unwrap_or(false);
        }
        if let Ok(v) = std::env::var("CODEX_SHIM_LOGGING__LEVEL") {
            config.logging.level = v;
        }
    }

    /// Resolve the actual upstream model name from a Codex-requested model.
    pub fn resolve_model(&self, requested: &str) -> String {
        self.models
            .map
            .get(requested)
            .cloned()
            .unwrap_or_else(|| requested.to_string())
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.server.base_path != "/v1" {
            anyhow::bail!(
                "server.base_path must be '/v1'. codex-shim routes are fixed and do not honor custom base paths."
            );
        }
        if self.features.is_some() {
            anyhow::bail!(
                "features.* is no longer accepted in shim config. Remove the 'features' block; runtime behavior is derived from provider capabilities and model catalog metadata."
            );
        }
        if self.models.catalog.is_empty() {
            anyhow::bail!(
                "models.catalog must define at least one model entry with slug and context_window for the shim-native /models endpoint"
            );
        }
        for model in &self.models.catalog {
            if model.slug.trim().is_empty() {
                anyhow::bail!("models.catalog entries must have a non-empty slug");
            }
            if model.context_window <= 0 {
                anyhow::bail!(
                    "models.catalog entry '{}' must have a positive context_window",
                    model.slug
                );
            }
        }
        let default_resolved = self.resolve_model(&self.models.default);
        if !self
            .models
            .catalog
            .iter()
            .any(|model| model.slug == default_resolved)
        {
            anyhow::bail!(
                "models.default resolves to '{}' but models.catalog does not contain that slug",
                default_resolved
            );
        }
        Ok(())
    }
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn default_config_path_for_home(home: Option<&Path>) -> Option<PathBuf> {
    home.map(|home| home.join(".codex-shim").join("config.yaml"))
}

pub(crate) fn default_config_path() -> Option<PathBuf> {
    default_config_path_for_home(home_dir().as_deref())
}

#[cfg(feature = "sqlite")]
fn default_state_store_path_for_home(home: Option<&Path>) -> PathBuf {
    match home {
        Some(home) => home.join(".codex-shim").join("store.db"),
        None => std::env::temp_dir().join(".codex-shim").join("store.db"),
    }
}

#[cfg(feature = "sqlite")]
pub(crate) fn default_state_store_path() -> PathBuf {
    default_state_store_path_for_home(home_dir().as_deref())
}

fn expand_tilde_with_home(path: &str, home: Option<&Path>) -> PathBuf {
    if path == "~"
        && let Some(home) = home
    {
        return home.to_path_buf();
    }

    if let Some(stripped) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
        && let Some(home) = home
    {
        return home.join(Path::new(stripped));
    }

    PathBuf::from(path)
}

pub fn expand_tilde(path: &str) -> PathBuf {
    expand_tilde_with_home(path, home_dir().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> Config {
        let mut config = Config::default();
        config.models.catalog = vec![CatalogModelSpec {
            slug: config.models.default.clone(),
            display_name: Some(config.models.default.clone()),
            description: None,
            context_window: 131072,
            tool_calling: Some(true),
            vision: Some(false),
            reasoning_levels: None,
            priority: None,
            base_instructions: None,
            auto_compact_token_limit: None,
            supports_search_tool: Some(false),
            supports_reasoning_summaries: Some(false),
            apply_patch_tool_type: None,
            supports_image_detail_original: Some(false),
        }];
        config
    }

    #[test]
    fn validate_accepts_minimal_catalog_config() {
        valid_config().validate().expect("config should validate");
    }

    #[test]
    fn validate_rejects_non_v1_base_path() {
        let mut config = valid_config();
        config.server.base_path = "/api".into();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("server.base_path must be '/v1'"));
    }

    #[test]
    fn validate_rejects_features_block() {
        let mut config = valid_config();
        config.features = Some(FeaturesConfig::default());
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("features.* is no longer accepted"));
    }

    #[test]
    fn validate_rejects_empty_catalog() {
        let mut config = valid_config();
        config.models.catalog.clear();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("models.catalog must define at least one model entry"));
    }

    #[test]
    fn validate_rejects_missing_default_model_in_catalog() {
        let mut config = valid_config();
        config.models.catalog[0].slug = "other-model".into();
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("models.default resolves to"));
    }

    #[test]
    fn default_config_path_uses_codex_shim_subdir() {
        let path = default_config_path_for_home(Some(Path::new("/home/tester")))
            .expect("default config path");
        assert_eq!(path, Path::new("/home/tester/.codex-shim/config.yaml"));
    }

    #[cfg(feature = "sqlite")]
    #[test]
    fn default_state_store_path_uses_codex_shim_subdir() {
        let path = default_state_store_path_for_home(Some(Path::new("/home/tester")));
        assert_eq!(path, Path::new("/home/tester/.codex-shim/store.db"));
    }

    #[test]
    fn expand_tilde_supports_home_root() {
        let path = expand_tilde_with_home("~", Some(Path::new("/home/tester")));
        assert_eq!(path, Path::new("/home/tester"));
    }

    #[test]
    fn expand_tilde_supports_forward_slash_paths() {
        let path = expand_tilde_with_home("~/config.yaml", Some(Path::new("/home/tester")));
        assert_eq!(path, Path::new("/home/tester/config.yaml"));
    }

    #[test]
    fn expand_tilde_supports_backslash_paths() {
        let path = expand_tilde_with_home("~\\config.yaml", Some(Path::new("/home/tester")));
        assert_eq!(path, Path::new("/home/tester/config.yaml"));
    }

    #[test]
    fn expand_tilde_leaves_plain_paths_unchanged() {
        let path = expand_tilde_with_home("config.yaml", Some(Path::new("/home/tester")));
        assert_eq!(path, Path::new("config.yaml"));
    }
}
