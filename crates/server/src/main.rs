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
        /// Project directory for Codex desktop project-scoped config.
        #[arg(long)]
        project_dir: Option<String>,
        /// Mark the project as trusted in the global Codex config.
        #[arg(long)]
        trust_project: bool,
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
    /// Validate desktop-oriented Codex project wiring without mutating config files
    Doctor {
        #[command(subcommand)]
        command: DoctorCommands,
    },
}

#[derive(Subcommand)]
enum DoctorCommands {
    /// Validate project-scoped Codex desktop config and trust wiring
    Desktop {
        /// Project directory that should contain .codex/config.toml
        #[arg(long)]
        project_dir: String,
        /// Codex home directory. Defaults to $CODEX_HOME or ~/.codex.
        #[arg(long)]
        codex_home: Option<String>,
        /// Provider profile override for catalog rendering.
        #[arg(long)]
        profile: Option<String>,
        /// Expected project provider ID.
        #[arg(long, default_value = "codex_shim")]
        provider_id: String,
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
            project_dir,
            trust_project,
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
            project_dir.as_deref(),
            trust_project,
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
        Some(Commands::Doctor { command }) => match command {
            DoctorCommands::Desktop {
                project_dir,
                codex_home,
                profile,
                provider_id,
            } => cmd_doctor_desktop(
                cli.config.as_deref(),
                &project_dir,
                codex_home.as_deref(),
                profile.as_deref(),
                &provider_id,
            ),
        },
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
    project_dir: Option<&str>,
    trust_project: bool,
    catalog_path: Option<&str>,
    base_url: &str,
    env_key: Option<&str>,
    web_search: Option<&str>,
) -> anyhow::Result<()> {
    if trust_project && project_dir.is_none() {
        bail!("--trust-project requires --project-dir");
    }

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
    let effective_web_search =
        resolve_install_web_search_mode(web_search, &caps, &config.models.catalog)?;

    if let Some(project_dir) = project_dir {
        if provider_id != "codex_shim" {
            bail!(
                "desktop project installs require provider_id 'codex_shim' to keep resume/history stable"
            );
        }
        if catalog_path.is_some() {
            bail!(
                "--catalog-path is not supported with --project-dir; desktop installs use a stable in-project catalog path"
            );
        }

        let project_dir = resolve_project_dir(project_dir)?;
        let project_config_path = project_config_path(&project_dir);
        let project_catalog_path = project_catalog_path(&project_dir)?;
        if let Some(parent) = project_catalog_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create project model catalog directory at {}",
                    parent.display()
                )
            })?;
        }

        write_catalog_json(&project_catalog_path, &catalog)?;
        update_codex_config_toml(
            &project_config_path,
            provider_id,
            target_model,
            &project_catalog_path,
            base_url,
            env_key,
            effective_web_search,
        )?;

        let mut trusted = false;
        let mut global_config_path = None;
        if trust_project {
            let codex_home = resolve_codex_home(codex_home)?;
            std::fs::create_dir_all(&codex_home).with_context(|| {
                format!("failed to create CODEX_HOME at {}", codex_home.display())
            })?;
            let path = codex_home.join("config.toml");
            update_project_trust_entry(&path, &project_dir)?;
            trusted = true;
            global_config_path = Some(path);
        }

        println!(
            "Wrote project model catalog: {}",
            project_catalog_path.display()
        );
        println!(
            "Updated project Codex config: {}",
            project_config_path.display()
        );
        println!(
            "Activated provider '{}' with model '{}' for desktop project '{}'.",
            provider_id,
            target_model,
            project_dir.display()
        );
        if let Some(path) = global_config_path {
            println!(
                "Marked project as trusted in global Codex config: {}",
                path.display()
            );
        } else {
            println!(
                "Desktop only reads project .codex/config.toml after the project is trusted. Re-run with --trust-project or trust it manually."
            );
        }
        println!("Restart Codex desktop to pick up the new project catalog.");
        if !trusted {
            println!(
                "History/resume is only guaranteed for shim-managed threads opened from this trusted project config."
            );
        }
        return Ok(());
    }

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
        effective_web_search,
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

fn cmd_doctor_desktop(
    config_path: Option<&str>,
    project_dir: &str,
    codex_home: Option<&str>,
    profile: Option<&str>,
    provider_id: &str,
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
        "shim config is not ready for desktop validation; fix models.catalog in config.yaml first"
    })?;

    let caps = match profile {
        Some(profile) => explicit_profile_caps(profile)?,
        None => configured_profile_caps(&config),
    };
    let report = build_desktop_doctor_report(
        &config,
        &caps,
        &resolve_project_dir(project_dir)?,
        codex_home,
        provider_id,
    )?;
    report.print();
    if report.has_unsupported() {
        bail!("desktop doctor found unsupported configuration issues");
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DesktopCheckStatus {
    Supported,
    Gated,
    Unsupported,
}

impl DesktopCheckStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Gated => "gated",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopCheck {
    status: DesktopCheckStatus,
    subject: &'static str,
    detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DesktopDoctorReport {
    checks: Vec<DesktopCheck>,
}

impl DesktopDoctorReport {
    fn has_unsupported(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == DesktopCheckStatus::Unsupported)
    }

    fn print(&self) {
        for check in &self.checks {
            println!(
                "[{}] {}: {}",
                check.status.label(),
                check.subject,
                check.detail
            );
        }
    }
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

fn resolve_install_web_search_mode<'a>(
    requested: Option<&'a str>,
    caps: &ProviderCapabilities,
    catalog: &[CatalogModelSpec],
) -> anyhow::Result<Option<&'a str>> {
    match requested {
        Some(mode @ ("disabled" | "cached" | "live")) => {
            if mode != "disabled" && !catalog_supports_web_search(caps, catalog) {
                bail!(
                    "web_search mode '{}' requires a Responses-capable profile with hosted web search enabled and every advertised catalog model to set supports_search_tool = true",
                    mode
                );
            }
            Ok(Some(mode))
        }
        Some(other) => bail!(
            "invalid web_search mode '{}'; expected one of: disabled, cached, live",
            other
        ),
        None if catalog_supports_web_search(caps, catalog) => Ok(None),
        None => Ok(Some("disabled")),
    }
}

fn catalog_supports_web_search(caps: &ProviderCapabilities, catalog: &[CatalogModelSpec]) -> bool {
    caps.supports_hosted_web_search
        && !catalog.is_empty()
        && catalog
            .iter()
            .all(|spec| spec.supports_search_tool.unwrap_or(false))
}

fn resolve_project_dir(project_dir: &str) -> anyhow::Result<PathBuf> {
    let path = absolutize(&expand_tilde(project_dir))?;
    let metadata = std::fs::metadata(&path)
        .with_context(|| format!("failed to read project directory {}", path.display()))?;
    if !metadata.is_dir() {
        bail!("project path '{}' is not a directory", path.display());
    }
    Ok(path)
}

fn project_config_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".codex").join("config.toml")
}

fn project_catalog_path(project_dir: &Path) -> anyhow::Result<PathBuf> {
    absolutize(
        &project_dir
            .join(".codex")
            .join("codex-shim")
            .join("model-catalog.json"),
    )
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

fn update_project_trust_entry(global_config_path: &Path, project_dir: &Path) -> anyhow::Result<()> {
    let (existing, mut doc) = read_toml_document(global_config_path)?;
    let projects = ensure_table(doc.as_table_mut(), "projects", "projects")?;
    let project_key = project_dir.to_string_lossy();
    let entry = ensure_table(projects, project_key.as_ref(), "projects.<path>")?;
    entry["trust_level"] = value("trusted");
    write_toml_document(global_config_path, existing.as_deref(), &doc)?;
    Ok(())
}

fn build_desktop_doctor_report(
    config: &Config,
    caps: &ProviderCapabilities,
    project_dir: &Path,
    codex_home: Option<&str>,
    provider_id: &str,
) -> anyhow::Result<DesktopDoctorReport> {
    let mut checks = Vec::new();
    let project_config = project_config_path(project_dir);
    let expected_catalog_path = project_catalog_path(project_dir)?;
    let expected_catalog = build_model_catalog(&config.models.catalog, caps);
    let expected_search_support = catalog_supports_web_search(caps, &config.models.catalog);

    if provider_id != "codex_shim" {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Unsupported,
            subject: "provider_id",
            detail: format!(
                "desktop project support requires provider_id 'codex_shim'; got '{}'",
                provider_id
            ),
        });
    } else {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Supported,
            subject: "provider_id",
            detail: "desktop project installs keep a stable provider identity for shim-managed history/resume".to_string(),
        });
    }

    if project_config.exists() {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Supported,
            subject: "project_config",
            detail: format!("found project config at {}", project_config.display()),
        });

        let project_toml = std::fs::read_to_string(&project_config)
            .with_context(|| format!("failed to read {}", project_config.display()))?;
        let project_doc = project_toml
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse {}", project_config.display()))?;

        let actual_provider = project_doc["model_provider"].as_str();
        if actual_provider == Some(provider_id) {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Supported,
                subject: "project_model_provider",
                detail: format!("project config uses stable provider '{}'", provider_id),
            });
        } else {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_model_provider",
                detail: format!(
                    "project config model_provider is {:?}; expected '{}'",
                    actual_provider, provider_id
                ),
            });
        }

        let active_model = project_doc["model"].as_str();
        match active_model {
            Some(model) if config.models.catalog.iter().any(|spec| spec.slug == model) => {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Supported,
                    subject: "project_model",
                    detail: format!(
                        "active project model '{}' is present in shim models.catalog",
                        model
                    ),
                });
            }
            Some(model) => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_model",
                detail: format!(
                    "project config model '{}' is not present in shim models.catalog",
                    model
                ),
            }),
            None => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_model",
                detail: "project config is missing top-level model".to_string(),
            }),
        }

        let actual_catalog = project_doc["model_catalog_json"].as_str();
        if actual_catalog == Some(expected_catalog_path.to_string_lossy().as_ref()) {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Supported,
                subject: "project_model_catalog_path",
                detail: format!(
                    "project config points at stable in-project catalog {}",
                    expected_catalog_path.display()
                ),
            });
        } else {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_model_catalog_path",
                detail: format!(
                    "project config model_catalog_json is {:?}; expected {}",
                    actual_catalog,
                    expected_catalog_path.display()
                ),
            });
        }

        let provider = &project_doc["model_providers"][provider_id];
        if provider.is_none() {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_provider_block",
                detail: format!(
                    "missing [model_providers.{}] block in project config",
                    provider_id
                ),
            });
        } else {
            let wire_api = provider["wire_api"].as_str();
            if wire_api == Some("responses") {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Supported,
                    subject: "wire_api",
                    detail: "project provider uses wire_api = \"responses\"".to_string(),
                });
            } else {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Unsupported,
                    subject: "wire_api",
                    detail: format!(
                        "project provider wire_api is {:?}; expected \"responses\"",
                        wire_api
                    ),
                });
            }

            let supports_websockets = provider["supports_websockets"].as_bool();
            if supports_websockets == Some(false) {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Supported,
                    subject: "supports_websockets",
                    detail: "project provider keeps supports_websockets = false".to_string(),
                });
            } else {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Unsupported,
                    subject: "supports_websockets",
                    detail: format!(
                        "project provider supports_websockets is {:?}; expected false",
                        supports_websockets
                    ),
                });
            }
        }

        let web_search = project_doc["web_search"].as_str();
        match web_search {
            Some("disabled") => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Supported,
                subject: "web_search",
                detail: "top-level web_search is disabled".to_string(),
            }),
            Some(mode @ ("cached" | "live")) if expected_search_support => {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Supported,
                    subject: "web_search",
                    detail: format!(
                        "top-level web_search = '{}' is consistent with the shim profile and catalog",
                        mode
                    ),
                });
            }
            Some(mode @ ("cached" | "live")) => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "web_search",
                detail: format!(
                    "top-level web_search = '{}' but the active shim profile/catalog does not advertise hosted web search for every model",
                    mode
                ),
            }),
            Some(other) => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "web_search",
                detail: format!(
                    "top-level web_search = '{}' is invalid; expected disabled, cached, or live",
                    other
                ),
            }),
            None => checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "web_search",
                detail: "project config is missing top-level web_search".to_string(),
            }),
        }
    } else {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Unsupported,
            subject: "project_config",
            detail: format!(
                "missing project config {}; run 'codex-shim install-codex-config --project-dir {}'",
                project_config.display(),
                project_dir.display()
            ),
        });
    }

    if expected_catalog_path.exists() {
        let actual_catalog =
            std::fs::read_to_string(&expected_catalog_path).with_context(|| {
                format!(
                    "failed to read project catalog {}",
                    expected_catalog_path.display()
                )
            })?;
        let actual_catalog: ModelsResponse =
            serde_json::from_str(&actual_catalog).with_context(|| {
                format!(
                    "failed to parse project catalog {}",
                    expected_catalog_path.display()
                )
            })?;
        if actual_catalog == expected_catalog {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Supported,
                subject: "project_catalog",
                detail: "project catalog matches the current shim config and provider profile"
                    .to_string(),
            });
        } else {
            checks.push(DesktopCheck {
                status: DesktopCheckStatus::Unsupported,
                subject: "project_catalog",
                detail: "project catalog differs from the current shim config or provider profile"
                    .to_string(),
            });
        }
    } else {
        checks.push(DesktopCheck {
            status: DesktopCheckStatus::Unsupported,
            subject: "project_catalog",
            detail: format!(
                "missing project catalog {}",
                expected_catalog_path.display()
            ),
        });
    }

    match resolve_codex_home(codex_home) {
        Ok(codex_home) => {
            let global_config = codex_home.join("config.toml");
            if global_config.exists() {
                let global_toml = std::fs::read_to_string(&global_config)
                    .with_context(|| format!("failed to read {}", global_config.display()))?;
                let global_doc = global_toml
                    .parse::<DocumentMut>()
                    .with_context(|| format!("failed to parse {}", global_config.display()))?;
                let trust_level =
                    global_doc["projects"][project_dir.to_string_lossy().as_ref()]["trust_level"]
                        .as_str();
                if trust_level == Some("trusted") {
                    checks.push(DesktopCheck {
                        status: DesktopCheckStatus::Supported,
                        subject: "project_trust",
                        detail: format!(
                            "global Codex config trusts project '{}'",
                            project_dir.display()
                        ),
                    });
                } else {
                    checks.push(DesktopCheck {
                        status: DesktopCheckStatus::Unsupported,
                        subject: "project_trust",
                        detail: format!(
                            "global Codex config does not mark '{}' as trusted; run install-codex-config --project-dir '{}' --trust-project",
                            project_dir.display(),
                            project_dir.display()
                        ),
                    });
                }
            } else {
                checks.push(DesktopCheck {
                    status: DesktopCheckStatus::Unsupported,
                    subject: "project_trust",
                    detail: format!(
                        "missing global Codex config {}; desktop cannot trust project-scoped config without it",
                        global_config.display()
                    ),
                });
            }
        }
        Err(err) => checks.push(DesktopCheck {
            status: DesktopCheckStatus::Unsupported,
            subject: "project_trust",
            detail: format!("could not resolve CODEX_HOME for trust validation: {err}"),
        }),
    }

    checks.push(DesktopCheck {
        status: DesktopCheckStatus::Gated,
        subject: "legacy_non_shim_threads",
        detail: "old non-shim desktop threads may still fail to resume with their original provider context; this depends on Codex desktop thread restoration behavior rather than codex-shim".to_string(),
    });

    Ok(DesktopDoctorReport { checks })
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
    let (existing, mut doc) = read_toml_document(path)?;

    merge_codex_config_document(
        &mut doc,
        provider_id,
        model,
        catalog_path,
        base_url,
        env_key,
        web_search,
    )?;
    write_toml_document(path, existing.as_deref(), &doc)?;
    Ok(())
}

fn read_toml_document(path: &Path) -> anyhow::Result<(Option<String>, DocumentMut)> {
    let existing = match std::fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };

    let doc = if existing.as_deref().unwrap_or_default().trim().is_empty() {
        DocumentMut::new()
    } else {
        existing
            .as_deref()
            .unwrap_or_default()
            .parse::<DocumentMut>()
            .with_context(|| format!("failed to parse {}", path.display()))?
    };
    Ok((existing, doc))
}

fn write_toml_document(
    path: &Path,
    existing: Option<&str>,
    doc: &DocumentMut,
) -> anyhow::Result<bool> {
    let rendered = normalize_toml(doc.to_string());
    if existing
        .map(|content| normalize_toml(content.to_string()) == rendered)
        .unwrap_or(false)
    {
        return Ok(false);
    }

    if existing.is_some() {
        rotate_config_backups(path)?;
    } else if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create config directory {}", parent.display()))?;
    }
    std::fs::write(path, rendered)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

fn normalize_toml(mut rendered: String) -> String {
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
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
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "codex-shim-{label}-{}-{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn sample_catalog_spec(supports_search_tool: bool) -> CatalogModelSpec {
        CatalogModelSpec {
            slug: "mock-model".into(),
            display_name: Some("mock-model".into()),
            description: None,
            context_window: 131072,
            tool_calling: Some(true),
            vision: Some(false),
            reasoning_levels: Some(vec!["high".into()]),
            priority: Some(10),
            base_instructions: Some(String::new()),
            auto_compact_token_limit: None,
            supports_search_tool: Some(supports_search_tool),
            supports_reasoning_summaries: Some(false),
            apply_patch_tool_type: None,
            supports_image_detail_original: Some(false),
        }
    }

    fn sample_config(provider_kind: &str, supports_search_tool: bool) -> Config {
        let mut config = Config::default();
        config.provider.kind = provider_kind.into();
        config.models.default = "mock-model".into();
        config.models.catalog = vec![sample_catalog_spec(supports_search_tool)];
        config
    }

    fn write_test_shim_config(
        dir: &Path,
        provider_kind: &str,
        supports_search_tool: bool,
    ) -> PathBuf {
        let path = dir.join("config.yaml");
        let supports_search_tool = if supports_search_tool {
            "true"
        } else {
            "false"
        };
        let yaml = format!(
            r#"
server:
  listen: "127.0.0.1:8787"
  base_path: "/v1"
upstream:
  base_url: "https://api.example.com"
  api_key_env: "EXAMPLE_API_KEY"
provider:
  kind: "{provider_kind}"
models:
  default: "mock-model"
  catalog:
    - slug: "mock-model"
      display_name: "mock-model"
      context_window: 131072
      tool_calling: true
      vision: false
      reasoning_levels: ["high"]
      supports_search_tool: {supports_search_tool}
state:
  backend: "memory"
logging:
  level: "info"
"#
        );
        std::fs::write(&path, yaml.trim()).expect("write shim config");
        path
    }

    fn doctor_report_subject<'a>(
        report: &'a DesktopDoctorReport,
        subject: &str,
    ) -> &'a DesktopCheck {
        report
            .checks
            .iter()
            .find(|check| check.subject == subject)
            .expect("missing doctor check")
    }

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
        assert_eq!(
            path,
            codex_home.join("codex-shim").join("model-catalog.json")
        );
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
    fn install_project_mode_writes_project_config_and_trust() {
        let root = unique_temp_dir("project-install");
        let project_dir = root.join("repo");
        let codex_home = root.join("codex-home");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        let shim_config = write_test_shim_config(&root, "deepseek-chat", false);

        cmd_install_codex_config(
            Some(shim_config.to_string_lossy().as_ref()),
            None,
            None,
            "codex_shim",
            Some(codex_home.to_string_lossy().as_ref()),
            Some(project_dir.to_string_lossy().as_ref()),
            true,
            None,
            "http://127.0.0.1:8787/v1",
            Some("LOCAL_SHIM_TOKEN"),
            None,
        )
        .expect("project install");

        let project_config = project_config_path(&project_dir);
        let project_catalog = project_catalog_path(&project_dir).expect("project catalog path");
        assert!(project_config.exists(), "project config should exist");
        assert!(project_catalog.exists(), "project catalog should exist");

        let project_doc = std::fs::read_to_string(&project_config)
            .expect("read project config")
            .parse::<DocumentMut>()
            .expect("parse project config");
        assert_eq!(project_doc["model_provider"].as_str(), Some("codex_shim"));
        assert_eq!(
            project_doc["model_catalog_json"].as_str(),
            Some(project_catalog.to_string_lossy().as_ref())
        );
        assert_eq!(project_doc["web_search"].as_str(), Some("disabled"));
        assert_eq!(
            project_doc["model_providers"]["codex_shim"]["wire_api"].as_str(),
            Some("responses")
        );
        assert_eq!(
            project_doc["model_providers"]["codex_shim"]["supports_websockets"].as_bool(),
            Some(false)
        );

        let global_doc = std::fs::read_to_string(codex_home.join("config.toml"))
            .expect("read global config")
            .parse::<DocumentMut>()
            .expect("parse global config");
        assert_eq!(
            global_doc["projects"][project_dir.to_string_lossy().as_ref()]["trust_level"].as_str(),
            Some("trusted")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_project_mode_is_idempotent_and_keeps_stable_paths() {
        let root = unique_temp_dir("project-idempotent");
        let project_dir = root.join("repo");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let shim_config = write_test_shim_config(&root, "deepseek-chat", false);

        cmd_install_codex_config(
            Some(shim_config.to_string_lossy().as_ref()),
            None,
            None,
            "codex_shim",
            None,
            Some(project_dir.to_string_lossy().as_ref()),
            false,
            None,
            "http://127.0.0.1:8787/v1",
            None,
            None,
        )
        .expect("first install");
        cmd_install_codex_config(
            Some(shim_config.to_string_lossy().as_ref()),
            None,
            None,
            "codex_shim",
            None,
            Some(project_dir.to_string_lossy().as_ref()),
            false,
            None,
            "http://127.0.0.1:8787/v1",
            None,
            None,
        )
        .expect("second install");

        let project_config = project_config_path(&project_dir);
        let project_doc = std::fs::read_to_string(&project_config)
            .expect("read project config")
            .parse::<DocumentMut>()
            .expect("parse project config");
        assert_eq!(project_doc["model_provider"].as_str(), Some("codex_shim"));
        assert_eq!(
            project_doc["model_catalog_json"].as_str(),
            Some(
                project_catalog_path(&project_dir)
                    .expect("project catalog path")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(
            !backup_path(&project_config, 0).exists(),
            "idempotent rerun should not rotate backups"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_project_mode_rejects_non_default_provider_id() {
        let root = unique_temp_dir("project-provider-id");
        let project_dir = root.join("repo");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        let shim_config = write_test_shim_config(&root, "deepseek-chat", false);

        let err = cmd_install_codex_config(
            Some(shim_config.to_string_lossy().as_ref()),
            None,
            None,
            "custom_provider",
            None,
            Some(project_dir.to_string_lossy().as_ref()),
            false,
            None,
            "http://127.0.0.1:8787/v1",
            None,
            None,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("provider_id 'codex_shim'"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_install_web_search_mode_rejects_unsupported_live() {
        let config = sample_config("deepseek-chat", false);
        let caps = configured_profile_caps(&config);
        let err = resolve_install_web_search_mode(Some("live"), &caps, &config.models.catalog)
            .unwrap_err();
        assert!(err.to_string().contains("hosted web search"));
    }

    #[test]
    fn desktop_doctor_reports_supported_project_install() {
        let root = unique_temp_dir("desktop-doctor-supported");
        let project_dir = root.join("repo");
        let codex_home = root.join("codex-home");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        let shim_config = write_test_shim_config(&root, "deepseek-chat", false);

        cmd_install_codex_config(
            Some(shim_config.to_string_lossy().as_ref()),
            None,
            None,
            "codex_shim",
            Some(codex_home.to_string_lossy().as_ref()),
            Some(project_dir.to_string_lossy().as_ref()),
            true,
            None,
            "http://127.0.0.1:8787/v1",
            None,
            None,
        )
        .expect("project install");

        let config = sample_config("deepseek-chat", false);
        let caps = configured_profile_caps(&config);
        let report = build_desktop_doctor_report(
            &config,
            &caps,
            &project_dir,
            Some(codex_home.to_string_lossy().as_ref()),
            "codex_shim",
        )
        .expect("doctor report");

        assert!(!report.has_unsupported(), "report: {:?}", report);
        assert_eq!(
            doctor_report_subject(&report, "project_trust").status,
            DesktopCheckStatus::Supported
        );
        assert_eq!(
            doctor_report_subject(&report, "legacy_non_shim_threads").status,
            DesktopCheckStatus::Gated
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn desktop_doctor_reports_untrusted_project_as_unsupported() {
        let root = unique_temp_dir("desktop-doctor-untrusted");
        let project_dir = root.join("repo");
        let codex_home = root.join("codex-home");
        std::fs::create_dir_all(&project_dir).expect("create project dir");
        std::fs::create_dir_all(&codex_home).expect("create codex home");
        let shim_config = write_test_shim_config(&root, "deepseek-chat", false);

        cmd_install_codex_config(
            Some(shim_config.to_string_lossy().as_ref()),
            None,
            None,
            "codex_shim",
            Some(codex_home.to_string_lossy().as_ref()),
            Some(project_dir.to_string_lossy().as_ref()),
            false,
            None,
            "http://127.0.0.1:8787/v1",
            None,
            None,
        )
        .expect("project install");

        let config = sample_config("deepseek-chat", false);
        let caps = configured_profile_caps(&config);
        let report = build_desktop_doctor_report(
            &config,
            &caps,
            &project_dir,
            Some(codex_home.to_string_lossy().as_ref()),
            "codex_shim",
        )
        .expect("doctor report");

        assert!(report.has_unsupported(), "report: {:?}", report);
        assert_eq!(
            doctor_report_subject(&report, "project_trust").status,
            DesktopCheckStatus::Unsupported
        );

        let _ = std::fs::remove_dir_all(&root);
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
