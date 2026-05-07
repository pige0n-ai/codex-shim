use std::path::{Path, PathBuf};

use anyhow::{Context, anyhow, bail};
use clap::{Parser, Subcommand};
use codex_shim::{config::Config, provider_profile_config::ProviderProfileConfig};
use protocol::{
    models::{CatalogModelSpec, ModelsResponse, build_model_catalog},
    provider_caps::ProviderCapabilities,
};
use toml_edit::{DocumentMut, Item, Table, value};

// ── CLI ──────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "codex-shim",
    version,
    about = "Local Responses API adapter for Codex custom model providers"
)]
struct Cli {
    /// Path to config YAML file
    #[arg(short, long, global = true)]
    config: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,

    // Server flags (used when no subcommand is given)
    /// Listen address for the local shim server.
    #[arg(long)]
    listen: Option<String>,
    /// Provider profile to emulate, such as deepseek-chat or openrouter-responses.
    #[arg(long)]
    provider: Option<String>,
    /// Base URL of the upstream provider API.
    #[arg(long)]
    upstream_base: Option<String>,
    /// Environment variable name that holds the upstream API key.
    #[arg(long)]
    upstream_key_env: Option<String>,
    /// Default upstream model slug to serve through the shim.
    #[arg(long)]
    model: Option<String>,
    /// State backend to use, for example memory or sqlite.
    #[arg(long)]
    state: Option<String>,
    /// Toggle provider reasoning defaults: enabled or disabled.
    #[arg(long)]
    thinking: Option<String>,
    /// Default reasoning effort to advertise to Codex.
    #[arg(long)]
    reasoning_effort: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate a model catalog JSON for a provider profile
    GenerateCatalog {
        /// Provider profile name (e.g. "deepseek-chat", "vllm-responses")
        #[arg(long, default_value = "deepseek-chat")]
        profile: String,
        /// Model slug (e.g. "deepseek-v4-pro")
        #[arg(long)]
        model: String,
        /// Context window tokens
        #[arg(long, default_value = "131072")]
        context_window: i64,
        /// Whether the model supports function/tool calling
        #[arg(long)]
        tool_calling: Option<bool>,
        /// Whether the model supports vision/image inputs
        #[arg(long)]
        vision: Option<bool>,
        /// Reasoning effort levels (comma-separated, e.g. "high,xhigh")
        #[arg(long)]
        reasoning_levels: Option<String>,
    },
    /// Write a startup model catalog into CODEX_HOME and update Codex config.toml
    #[command(visible_alias = "inject-codex-config")]
    InstallCodexConfig {
        /// Codex default model slug. Defaults to models.default from the shim config.
        #[arg(long)]
        model: Option<String>,
        /// Override the provider profile used to render model metadata.
        #[arg(long)]
        profile: Option<String>,
        /// Custom provider ID to create under [model_providers].
        #[arg(long, default_value = "codex_shim")]
        provider_id: String,
        /// Codex home directory. Defaults to $CODEX_HOME or ~/.codex.
        #[arg(long)]
        codex_home: Option<String>,
        /// Path to the startup model catalog JSON. Relative paths are resolved inside CODEX_HOME.
        #[arg(long)]
        catalog_path: Option<String>,
        /// Base URL Codex should call for the local shim.
        #[arg(long, default_value = "http://127.0.0.1:8787/v1")]
        base_url: String,
        /// Optional env var Codex should use for the shim bearer token.
        #[arg(long)]
        env_key: Option<String>,
        /// Optional Codex top-level web_search mode: disabled, cached, or live.
        #[arg(long)]
        web_search: Option<String>,
    },
    /// Explain what a model catalog JSON means to Codex
    ExplainCatalog {
        /// Path to the model catalog JSON file
        path: String,
    },
    /// Probe an upstream endpoint and report detected capabilities
    Probe {
        /// Provider profile name or "auto"
        #[arg(long, default_value = "auto")]
        profile: String,
        /// Upstream base URL
        #[arg(long, default_value = "http://127.0.0.1:8000/v1")]
        base_url: String,
        /// API key env var name
        #[arg(long)]
        api_key_env: Option<String>,
    },
}

// ── main ─────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::GenerateCatalog {
            profile,
            model,
            context_window,
            tool_calling,
            vision,
            reasoning_levels,
        }) => cmd_generate_catalog(
            &profile,
            &model,
            context_window,
            tool_calling,
            vision,
            reasoning_levels.as_deref(),
        ),
        Some(Commands::InstallCodexConfig {
            model,
            profile,
            provider_id,
            codex_home,
            catalog_path,
            base_url,
            env_key,
            web_search,
        }) => cmd_install_codex_config(
            cli.config.as_deref(),
            model.as_deref(),
            profile.as_deref(),
            &provider_id,
            codex_home.as_deref(),
            catalog_path.as_deref(),
            &base_url,
            env_key.as_deref(),
            web_search.as_deref(),
        ),
        Some(Commands::ExplainCatalog { path }) => cmd_explain_catalog(&path),
        Some(Commands::Probe {
            profile: _,
            base_url,
            api_key_env,
        }) => cmd_probe(&base_url, api_key_env.as_deref()).await,
        None => run_server(&cli).await,
    }
}

// ── Server ───────────────────────────────────────────────────────

async fn run_server(cli: &Cli) -> anyhow::Result<()> {
    let mut config = Config::load(cli.config.as_deref())?;

    if let Some(listen) = &cli.listen {
        config.server.listen = listen.clone();
    }
    if let Some(provider) = &cli.provider {
        config.provider.kind = provider.clone();
    }
    if let Some(base) = &cli.upstream_base {
        config.upstream.base_url = base.clone();
    }
    if let Some(key_env) = &cli.upstream_key_env {
        config.upstream.api_key_env = key_env.clone();
    }
    if let Some(model) = &cli.model {
        config.models.default = model.clone();
        if config.models.catalog.len() == 1 {
            config.models.catalog[0].slug = model.clone();
            if config.models.catalog[0].display_name.is_none() {
                config.models.catalog[0].display_name = Some(model.clone());
            }
        }
    }
    if let Some(state) = &cli.state {
        config.state.backend = state.clone();
    }
    if let Some(thinking) = &cli.thinking {
        config.reasoning.enabled = thinking == "enabled";
    }
    if let Some(effort) = &cli.reasoning_effort {
        config.reasoning.effort = effort.clone();
    }

    let log_level = match config.logging.level.as_str() {
        "debug" => tracing::Level::DEBUG,
        "info" => tracing::Level::INFO,
        "warn" => tracing::Level::WARN,
        "error" => tracing::Level::ERROR,
        _ => tracing::Level::INFO,
    };
    tracing_subscriber::fmt().with_max_level(log_level).init();

    if let Some(path) = &cli.config {
        check_config_permissions(path);
    }

    let listen_addr = config.server.listen.clone();
    tracing::info!(listen = %listen_addr, provider = %config.provider.kind, upstream = %config.upstream.base_url, "Starting codex-shim");
    let app = codex_shim::app(config)?;
    let listener = tokio::net::TcpListener::bind(&listen_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

// ── generate-catalog ─────────────────────────────────────────────

fn cmd_generate_catalog(
    profile: &str,
    model: &str,
    context_window: i64,
    tool_calling: Option<bool>,
    vision: Option<bool>,
    reasoning_levels: Option<&str>,
) -> anyhow::Result<()> {
    let caps = explicit_profile_caps(profile)?;
    let catalog = build_model_catalog(
        &[CatalogModelSpec {
            slug: model.to_string(),
            display_name: Some(model.to_string()),
            description: None,
            context_window,
            tool_calling,
            vision,
            reasoning_levels: reasoning_levels
                .map(|s| s.split(',').map(|e| e.trim().to_string()).collect()),
            priority: Some(10),
            base_instructions: Some(String::new()),
            auto_compact_token_limit: None,
            supports_search_tool: Some(false),
            supports_reasoning_summaries: Some(false),
            apply_patch_tool_type: None,
            supports_image_detail_original: Some(false),
        }],
        &caps,
    );

    println!("{}", serde_json::to_string_pretty(&catalog)?);
    Ok(())
}

// ── install-codex-config ─────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn cmd_install_codex_config(
    config_path: Option<&str>,
    model: Option<&str>,
    profile: Option<&str>,
    provider_id: &str,
    codex_home: Option<&str>,
    catalog_path: Option<&str>,
    base_url: &str,
    env_key: Option<&str>,
    web_search: Option<&str>,
) -> anyhow::Result<()> {
    let config = Config::load(config_path).with_context(|| {
        if let Some(path) = config_path {
            format!("failed to load shim config from {}", path)
        } else {
            "failed to load shim config from --config or the default ~/.codex-shim/config.yaml"
                .to_string()
        }
    })?;
    config.validate().with_context(|| {
        "shim config is not ready for Codex installation; fix models.catalog in config.yaml first"
    })?;

    let target_model = model.unwrap_or(&config.models.default);
    let available_models: Vec<&str> = config
        .models
        .catalog
        .iter()
        .map(|spec| spec.slug.as_str())
        .collect();
    if !available_models.contains(&target_model) {
        bail!(
            "model '{}' is not present in shim models.catalog. Available models: {}",
            target_model,
            available_models.join(", ")
        );
    }

    let caps = match profile {
        Some(profile) => explicit_profile_caps(profile)?,
        None => configured_profile_caps(&config),
    };
    let catalog = build_model_catalog(&config.models.catalog, &caps);

    let codex_home = resolve_codex_home(codex_home)?;
    std::fs::create_dir_all(&codex_home)
        .with_context(|| format!("failed to create CODEX_HOME at {}", codex_home.display()))?;

    let catalog_path = resolve_catalog_path(&codex_home, catalog_path)?;
    if let Some(parent) = catalog_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create model catalog directory at {}",
                parent.display()
            )
        })?;
    }

    let config_toml_path = codex_home.join("config.toml");
    write_catalog_json(&catalog_path, &catalog)?;
    update_codex_config_toml(
        &config_toml_path,
        provider_id,
        target_model,
        &catalog_path,
        base_url,
        env_key,
        web_search,
    )?;

    println!("Wrote model catalog: {}", catalog_path.display());
    println!("Updated Codex config: {}", config_toml_path.display());
    println!(
        "Activated provider '{}' with model '{}' for Codex.",
        provider_id, target_model
    );
    println!("Restart Codex to pick up the new startup catalog.");
    Ok(())
}

// ── explain-catalog ──────────────────────────────────────────────

fn cmd_explain_catalog(path: &str) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let catalog: serde_json::Value = serde_json::from_str(&content)?;
    let models = catalog["models"]
        .as_array()
        .ok_or_else(|| anyhow!("Missing 'models' array"))?;

    for model in models {
        let slug = model["slug"].as_str().unwrap_or("?");
        let shell = model["shell_type"].as_str().unwrap_or("unknown");
        let ctx = model["context_window"].as_i64().unwrap_or(0);
        let parallel = model["supports_parallel_tool_calls"]
            .as_bool()
            .unwrap_or(false);
        let reasoning = model["supports_reasoning_summaries"]
            .as_bool()
            .unwrap_or(false);
        let modalities: Vec<&str> = model["input_modalities"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        let web_search = model["web_search_tool_type"].as_str().unwrap_or("text");
        let patch = model["apply_patch_tool_type"].as_str();

        println!("Model: {slug}");
        println!("  Shell type:          {shell}");
        println!("  Context window:      {ctx} tokens");
        println!("  Parallel tool calls: {parallel}");
        println!("  Reasoning summaries: {reasoning}");
        println!("  Input modalities:    {}", modalities.join(", "));
        println!("  Web search:          {web_search}");
        println!("  Apply patch:         {}", patch.unwrap_or("<disabled>"));
        println!();
        println!("  Codex will:");
        if shell == "unified_exec" {
            println!("    ✓ Use exec_command / write_stdin shell");
        } else {
            println!("    ✗ No shell tool");
        }
        if parallel {
            println!("    ✓ Make parallel tool calls when beneficial");
        } else {
            println!("    ✗ Serial tool calls only");
        }
        if shell != "disabled" && patch.is_some() {
            println!("    ✓ Use apply_patch tool for code edits");
        } else {
            println!("    ✗ No apply_patch tool");
        }
        if web_search != "text" {
            println!("    ✓ Web search available");
        } else {
            println!("    ✗ Web search disabled (Chat API has no server-side web search)");
        }
        if reasoning {
            println!("    ✓ Reasoning summaries requested");
        } else {
            println!("    ✗ No reasoning summaries");
        }
        if modalities.contains(&"image") {
            println!("    ✓ Image inputs accepted");
        } else {
            println!("    ✗ Text-only inputs");
        }
        println!();
    }
    Ok(())
}

// ── probe ────────────────────────────────────────────────────────

async fn cmd_probe(base_url: &str, api_key_env: Option<&str>) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    let mut result = serde_json::json!({
        "base_url": base_url,
        "responses": false,
        "responses_stateful": false,
        "chat_completions": false,
        "streaming": "unknown",
        "function_tools": false,
        "parallel_tool_calls": "unknown",
        "reasoning_shape": "unknown",
        "usage_in_stream_final": false,
        "errors": []
    });

    // Probe /v1/chat/completions
    let chat_url = format!("{base_url}/chat/completions");
    let mut req = client.post(&chat_url).json(&serde_json::json!({
        "model": "probe",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1,
        "stream": false,
    }));
    if let Some(ref key_env) = api_key_env
        && let Ok(key) = std::env::var(key_env)
    {
        req = req.bearer_auth(key);
    }
    match req.send().await {
        Ok(_resp) => {
            result["chat_completions"] = serde_json::Value::Bool(true);
            if let Ok(body) = _resp.text().await
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&body)
                && json.get("choices").is_some()
            {
                result["chat_completions"] = serde_json::Value::Bool(true);
                if json.get("usage").is_some() {
                    result["usage_in_stream_final"] = serde_json::Value::Bool(true);
                }
            }
        }
        Err(e) => {
            result["errors"] = serde_json::json!([format!("chat_completions: {e}")]);
        }
    }

    // Probe /v1/responses
    let resp_url = format!("{base_url}/responses");
    let mut req = client.post(&resp_url).json(&serde_json::json!({
        "model": "probe",
        "input": "hi",
        "max_output_tokens": 1,
    }));
    if let Some(ref key_env) = api_key_env
        && let Ok(key) = std::env::var(key_env)
    {
        req = req.bearer_auth(key);
    }
    match req.send().await {
        Ok(_resp) => {
            if let Ok(body) = _resp.text().await
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&body)
            {
                if json.get("output").is_some() || json.get("status").is_some() {
                    result["responses"] = serde_json::Value::Bool(true);
                }
                if json.get("previous_response_id").is_some() || json.get("store").is_some() {
                    result["responses_stateful"] = serde_json::Value::Bool(true);
                }
                if let Some(output) = json.get("output").and_then(|o| o.as_array())
                    && output
                        .iter()
                        .any(|item| item.get("type").and_then(|v| v.as_str()) == Some("reasoning"))
                {
                    result["reasoning_shape"] = serde_json::json!("responses_reasoning_item");
                }
                if json.get("usage").is_some() {
                    result["usage_in_stream_final"] = serde_json::Value::Bool(true);
                }
            }
        }
        Err(e) => {
            if let Some(arr) = result["errors"].as_array() {
                let mut new_arr = arr.clone();
                new_arr.push(serde_json::json!(format!("responses: {e}")));
                result["errors"] = serde_json::Value::Array(new_arr);
            }
        }
    }

    // Probe /v1/models
    let models_url = format!("{base_url}/models");
    let mut req = client.get(&models_url);
    if let Some(ref key_env) = api_key_env
        && let Ok(key) = std::env::var(key_env)
    {
        req = req.bearer_auth(key);
    }
    if let Ok(resp) = req.send().await
        && let Ok(body) = resp.text().await
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(&body)
        && (json.get("data").is_some() || json.get("object").is_some())
    {
        // Valid models endpoint
    }

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

// ── helpers ──────────────────────────────────────────────────────

fn configured_profile_caps(config: &Config) -> ProviderCapabilities {
    let profile_cfg =
        config
            .provider
            .profile_config
            .clone()
            .unwrap_or_else(|| ProviderProfileConfig {
                profile: config.provider.kind.clone(),
                ..Default::default()
            });
    profile_cfg.build_profile().capabilities().clone()
}

fn explicit_profile_caps(profile: &str) -> anyhow::Result<ProviderCapabilities> {
    if !providers::is_supported_profile_name(profile) {
        bail!(
            "unknown provider profile '{}'. Supported profiles: {}",
            profile,
            providers::SUPPORTED_PROFILE_NAMES.join(", ")
        );
    }
    Ok(providers::create_profile(profile, None)
        .capabilities()
        .clone())
}

fn resolve_codex_home(codex_home: Option<&str>) -> anyhow::Result<PathBuf> {
    let path = match codex_home {
        Some(path) => expand_tilde(path),
        None => match std::env::var("CODEX_HOME") {
            Ok(path) => expand_tilde(&path),
            Err(_) => default_codex_home_dir().ok_or_else(|| {
                anyhow!("could not determine CODEX_HOME; set --codex-home or CODEX_HOME")
            })?,
        },
    };
    absolutize(&path)
}

fn default_codex_home_dir_for_home(home: Option<&Path>) -> Option<PathBuf> {
    home.map(|home| home.join(".codex"))
}

fn default_codex_home_dir() -> Option<PathBuf> {
    default_codex_home_dir_for_home(home_dir().as_deref())
}

fn resolve_catalog_path(codex_home: &Path, catalog_path: Option<&str>) -> anyhow::Result<PathBuf> {
    let candidate = match catalog_path {
        Some(path) => {
            let path = expand_tilde(path);
            if path.is_absolute() {
                path
            } else {
                codex_home.join(path)
            }
        }
        None => codex_home.join("codex-shim").join("model-catalog.json"),
    };
    absolutize(&candidate)
}

fn write_catalog_json(path: &Path, catalog: &ModelsResponse) -> anyhow::Result<()> {
    let mut rendered = serde_json::to_string_pretty(catalog)?;
    rendered.push('\n');
    std::fs::write(path, rendered)
        .with_context(|| format!("failed to write model catalog to {}", path.display()))
}

fn update_codex_config_toml(
    path: &Path,
    provider_id: &str,
    model: &str,
    catalog_path: &Path,
    base_url: &str,
    env_key: Option<&str>,
    web_search: Option<&str>,
) -> anyhow::Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };

    let mut doc = if existing.as_deref().unwrap_or_default().trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .as_deref()
            .unwrap_or_default()
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse {}", path.display()))?
    };

    merge_codex_config_document(
        &mut doc,
        provider_id,
        model,
        catalog_path,
        base_url,
        env_key,
        web_search,
    )?;

    if existing.is_some() {
        rotate_config_backups(path)?;
    }

    let mut rendered = doc.to_string();
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    std::fs::write(path, rendered).with_context(|| format!("failed to write {}", path.display()))
}

fn merge_codex_config_document(
    doc: &mut DocumentMut,
    provider_id: &str,
    model: &str,
    catalog_path: &Path,
    base_url: &str,
    env_key: Option<&str>,
    web_search: Option<&str>,
) -> anyhow::Result<()> {
    if provider_id.trim().is_empty() {
        bail!("provider_id must not be empty");
    }
    if matches!(
        provider_id,
        "openai" | "ollama" | "lmstudio" | "amazon-bedrock"
    ) {
        bail!(
            "provider_id '{}' is reserved by Codex; choose a different custom provider ID",
            provider_id
        );
    }

    let web_search = match web_search {
        Some(mode @ ("disabled" | "cached" | "live")) => Some(mode),
        Some(other) => bail!(
            "invalid web_search mode '{}'; expected one of: disabled, cached, live",
            other
        ),
        None => None,
    };

    doc["model_provider"] = value(provider_id);
    doc["model"] = value(model);
    doc["model_catalog_json"] = value(catalog_path.to_string_lossy().to_string());

    if let Some(mode) = web_search {
        doc["web_search"] = value(mode);
    } else if !doc.as_table().contains_key("web_search") {
        doc["web_search"] = value("disabled");
    }

    let model_providers = ensure_table(doc.as_table_mut(), "model_providers", "model_providers")?;
    let provider_path = format!("model_providers.{provider_id}");
    let provider = ensure_table(model_providers, provider_id, &provider_path)?;

    if env_key.is_some() && provider.contains_key("auth") {
        bail!(
            "{} already contains an auth block. Remove it or use a different --provider-id before installing env_key-based shim auth.",
            provider_path
        );
    }

    // Clean up the old, incorrect location if a user copied a previous example.
    let _ = provider.remove("model_catalog_json");

    provider["name"] = value("codex-shim");
    provider["base_url"] = value(base_url);
    provider["wire_api"] = value("responses");
    provider["supports_websockets"] = value(false);
    match env_key {
        Some(env_key) => {
            provider["env_key"] = value(env_key);
            provider["env_key_instructions"] =
                value(format!("Set {env_key} before starting Codex."));
        }
        None => {
            let _ = provider.remove("env_key");
            let _ = provider.remove("env_key_instructions");
        }
    }
    Ok(())
}

fn rotate_config_backups(path: &Path) -> anyhow::Result<()> {
    for index in (0..=3).rev() {
        let src = backup_path(path, index);
        if !src.exists() {
            continue;
        }

        if index == 3 {
            std::fs::remove_file(&src)
                .with_context(|| format!("failed to remove old backup {}", src.display()))?;
            continue;
        }

        let dst = backup_path(path, index + 1);
        if dst.exists() {
            std::fs::remove_file(&dst)
                .with_context(|| format!("failed to replace backup {}", dst.display()))?;
        }
        std::fs::rename(&src, &dst).with_context(|| {
            format!(
                "failed to rotate backup {} to {}",
                src.display(),
                dst.display()
            )
        })?;
    }

    let backup0 = backup_path(path, 0);
    std::fs::copy(path, &backup0).with_context(|| {
        format!(
            "failed to create config backup {} from {}",
            backup0.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn backup_path(path: &Path, index: u8) -> PathBuf {
    PathBuf::from(format!("{}.bak.{}", path.display(), index))
}

fn ensure_table<'a>(
    parent: &'a mut Table,
    key: &str,
    full_path: &str,
) -> anyhow::Result<&'a mut Table> {
    if parent.get(key).is_none() {
        parent.insert(key, Item::Table(Table::new()));
    }
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("{full_path} must be a TOML table"))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn expand_tilde(path: &str) -> PathBuf {
    match home_dir().as_deref() {
        Some(home) if path == "~" => home.to_path_buf(),
        Some(home) => {
            if let Some(stripped) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
                home.join(Path::new(stripped))
            } else {
                PathBuf::from(path)
            }
        }
        None => PathBuf::from(path),
    }
}

fn absolutize(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn check_config_permissions(path: &str) {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode();
        if mode & 0o002 != 0 {
            tracing::warn!(
                path,
                "Config file is world-writable. Consider: chmod 600 {path}"
            );
        }
    }

    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_codex_config_writes_top_level_catalog_pointer() {
        let mut doc = DocumentMut::new();
        let catalog = Path::new("/tmp/codex/model-catalog.json");

        merge_codex_config_document(
            &mut doc,
            "codex_shim",
            "deepseek-v4-pro",
            catalog,
            "http://127.0.0.1:8787/v1",
            Some("LOCAL_SHIM_TOKEN"),
            None,
        )
        .expect("merge should succeed");

        assert_eq!(doc["model_provider"].as_str(), Some("codex_shim"));
        assert_eq!(doc["model"].as_str(), Some("deepseek-v4-pro"));
        assert_eq!(
            doc["model_catalog_json"].as_str(),
            Some("/tmp/codex/model-catalog.json")
        );
        assert_eq!(doc["web_search"].as_str(), Some("disabled"));
        assert_eq!(
            doc["model_providers"]["codex_shim"]["base_url"].as_str(),
            Some("http://127.0.0.1:8787/v1")
        );
        assert_eq!(
            doc["model_providers"]["codex_shim"]["wire_api"].as_str(),
            Some("responses")
        );
        assert_eq!(
            doc["model_providers"]["codex_shim"]["supports_websockets"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn merge_codex_config_preserves_existing_web_search_when_not_overridden() {
        let mut doc = r#"
web_search = "live"
[model_providers.other]
name = "Other"
"#
        .parse::<DocumentMut>()
        .expect("parse TOML");

        merge_codex_config_document(
            &mut doc,
            "codex_shim",
            "deepseek-v4-pro",
            Path::new("/tmp/codex/model-catalog.json"),
            "http://127.0.0.1:8787/v1",
            Some("LOCAL_SHIM_TOKEN"),
            None,
        )
        .expect("merge should succeed");

        assert_eq!(doc["web_search"].as_str(), Some("live"));
        assert_eq!(
            doc["model_providers"]["other"]["name"].as_str(),
            Some("Other")
        );
    }

    #[test]
    fn merge_codex_config_removes_legacy_provider_local_catalog_path() {
        let mut doc = r#"
[model_providers.codex_shim]
name = "codex-shim"
model_catalog_json = "/wrong/place.json"
"#
        .parse::<DocumentMut>()
        .expect("parse TOML");

        merge_codex_config_document(
            &mut doc,
            "codex_shim",
            "deepseek-v4-pro",
            Path::new("/tmp/codex/model-catalog.json"),
            "http://127.0.0.1:8787/v1",
            Some("LOCAL_SHIM_TOKEN"),
            None,
        )
        .expect("merge should succeed");

        assert!(
            doc["model_providers"]["codex_shim"]
                .as_table()
                .is_some_and(|table| !table.contains_key("model_catalog_json"))
        );
        assert_eq!(
            doc["model_catalog_json"].as_str(),
            Some("/tmp/codex/model-catalog.json")
        );
    }

    #[test]
    fn merge_codex_config_rejects_existing_auth_block() {
        let mut doc = r#"
[model_providers.codex_shim.auth]
command = "/bin/true"
"#
        .parse::<DocumentMut>()
        .expect("parse TOML");

        let err = merge_codex_config_document(
            &mut doc,
            "codex_shim",
            "deepseek-v4-pro",
            Path::new("/tmp/codex/model-catalog.json"),
            "http://127.0.0.1:8787/v1",
            Some("LOCAL_SHIM_TOKEN"),
            None,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("auth block"));
    }

    #[test]
    fn default_codex_home_dir_uses_codex_subdir() {
        let path =
            default_codex_home_dir_for_home(Some(Path::new("/home/tester"))).expect("default path");
        assert_eq!(path, Path::new("/home/tester/.codex"));
    }

    #[test]
    fn resolve_catalog_path_defaults_inside_codex_home() {
        let codex_home = std::env::current_dir()
            .expect("current dir")
            .join("fixtures")
            .join("codex-home");
        let path = resolve_catalog_path(&codex_home, None).expect("default catalog path");
        assert_eq!(path, codex_home.join("codex-shim").join("model-catalog.json"));
    }

    #[test]
    fn merge_codex_config_can_omit_env_key_for_local_loopback() {
        let mut doc = DocumentMut::new();

        merge_codex_config_document(
            &mut doc,
            "codex_shim",
            "deepseek-v4-pro",
            Path::new("/tmp/codex/model-catalog.json"),
            "http://127.0.0.1:8787/v1",
            None,
            None,
        )
        .expect("merge should succeed");

        assert!(
            doc["model_providers"]["codex_shim"]
                .as_table()
                .is_some_and(|table| !table.contains_key("env_key"))
        );
        assert!(
            doc["model_providers"]["codex_shim"]
                .as_table()
                .is_some_and(|table| !table.contains_key("env_key_instructions"))
        );
    }

    #[test]
    fn rotate_config_backups_keeps_last_four_versions() {
        let dir =
            std::env::temp_dir().join(format!("codex-shim-backup-test-{}", std::process::id()));
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("config.toml");

        std::fs::write(&path, "version = 0\n").expect("write v0");
        rotate_config_backups(&path).expect("backup v0");
        std::fs::write(&path, "version = 1\n").expect("write v1");
        rotate_config_backups(&path).expect("backup v1");
        std::fs::write(&path, "version = 2\n").expect("write v2");
        rotate_config_backups(&path).expect("backup v2");
        std::fs::write(&path, "version = 3\n").expect("write v3");
        rotate_config_backups(&path).expect("backup v3");
        std::fs::write(&path, "version = 4\n").expect("write v4");
        rotate_config_backups(&path).expect("backup v4");

        assert_eq!(
            std::fs::read_to_string(backup_path(&path, 0)).expect("bak0"),
            "version = 4\n"
        );
        assert_eq!(
            std::fs::read_to_string(backup_path(&path, 1)).expect("bak1"),
            "version = 3\n"
        );
        assert_eq!(
            std::fs::read_to_string(backup_path(&path, 2)).expect("bak2"),
            "version = 2\n"
        );
        assert_eq!(
            std::fs::read_to_string(backup_path(&path, 3)).expect("bak3"),
            "version = 1\n"
        );
        assert!(!backup_path(&path, 4).exists());

        std::fs::remove_dir_all(&dir).expect("remove temp dir");
    }
}
