use clap::{Parser, Subcommand};
use protocol::models::{CatalogModelSpec, build_model_catalog};

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
        }) => {
            cmd_generate_catalog(
                &profile,
                &model,
                context_window,
                tool_calling,
                vision,
                reasoning_levels.as_deref(),
            );
            Ok(())
        }
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
    let mut config = codex_shim::config::Config::load(cli.config.as_deref())?;

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
) {
    use providers::create_profile;
    let prov = create_profile(profile, None);
    let caps = prov.capabilities();

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
        caps,
    );

    println!("{}", serde_json::to_string_pretty(&catalog).unwrap());
}

// ── explain-catalog ──────────────────────────────────────────────

fn cmd_explain_catalog(path: &str) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let catalog: serde_json::Value = serde_json::from_str(&content)?;
    let models = catalog["models"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Missing 'models' array"))?;

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
