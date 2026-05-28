pub mod mock_upstream;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use protocol::models::{CatalogModelSpec, build_model_catalog};
use serde::Deserialize;

// ── Codex Exec Runner ────────────────────────────────────────────

#[derive(Debug)]
pub struct CodexRunResult {
    pub status: std::process::ExitStatus,
    pub stdout_jsonl: Vec<serde_json::Value>,
    pub stderr: String,
    pub last_message: String,
}

#[derive(Debug, Clone)]
pub struct CodexExecOptions {
    pub sandbox: String,
    pub ephemeral: bool,
    pub resume_last: bool,
    pub json_output: bool,
}

impl Default for CodexExecOptions {
    fn default() -> Self {
        Self {
            sandbox: "read-only".into(),
            ephemeral: true,
            resume_last: false,
            json_output: true,
        }
    }
}

/// Run `codex exec` with a temporary CODEX_HOME, workdir, and prompt.
pub async fn run_codex_exec(
    codex_home: &Path,
    workdir: &Path,
    prompt: &str,
    envs: &[(&str, &str)],
) -> anyhow::Result<CodexRunResult> {
    run_codex_exec_with_options(
        codex_home,
        workdir,
        prompt,
        envs,
        &CodexExecOptions::default(),
    )
    .await
}

pub async fn run_codex_exec_with_options(
    codex_home: &Path,
    workdir: &Path,
    prompt: &str,
    envs: &[(&str, &str)],
    options: &CodexExecOptions,
) -> anyhow::Result<CodexRunResult> {
    let out_file = workdir.join(if options.resume_last {
        "last-message-resume.txt"
    } else {
        "last-message.txt"
    });
    let mut cmd = tokio::process::Command::new("codex");
    cmd.env("CODEX_HOME", codex_home)
        .env("LOCAL_SHIM_TOKEN", "local-shim-test-token")
        .env("RUST_LOG", "info")
        .arg("exec");

    if options.json_output {
        cmd.arg("--json");
    }
    cmd.arg("--skip-git-repo-check");
    if options.ephemeral {
        cmd.arg("--ephemeral");
    }
    cmd.arg("--sandbox")
        .arg(&options.sandbox)
        .arg("--color")
        .arg("never")
        .arg("--output-last-message")
        .arg(&out_file)
        .arg("--cd")
        .arg(workdir);

    if options.resume_last {
        cmd.arg("resume").arg("--last");
    }

    cmd.arg(prompt);

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let output = tokio::time::timeout(std::time::Duration::from_secs(180), cmd.output()).await??;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_jsonl: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .collect();

    let last_message = std::fs::read_to_string(&out_file).unwrap_or_default();

    Ok(CodexRunResult {
        status: output.status,
        stdout_jsonl,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        last_message,
    })
}

// ── Model Catalog & Codex Home ───────────────────────────────────

/// Shared model catalog builder.
fn build_catalog_json(model: &str, with_reasoning: bool) -> serde_json::Value {
    let reasoning_levels = if with_reasoning {
        Some(vec!["high".to_string()])
    } else {
        None
    };
    let caps = protocol::provider_caps::ProviderCapabilities {
        supports_function_tools: true,
        supports_parallel_tool_calls: true,
        supports_reasoning_effort: with_reasoning,
        ..Default::default()
    };
    serde_json::to_value(build_model_catalog(
        &[CatalogModelSpec {
            slug: model.to_string(),
            display_name: Some(model.to_string()),
            description: None,
            context_window: 131072,
            tool_calling: Some(true),
            vision: Some(false),
            reasoning_levels,
            priority: Some(10),
            base_instructions: None,
            auto_compact_token_limit: None,
            supports_search_tool: Some(false),
            supports_reasoning_summaries: Some(false),
            apply_patch_tool_type: None,
            apply_patch_upstream_tool_type: None,
            apply_patch_upstream_strict: None,
            supports_image_detail_original: Some(false),
        }],
        &caps,
    ))
    .unwrap()
}

fn write_codex_home_dir(
    home: &Path,
    provider_id: &str,
    model: &str,
    catalog: &serde_json::Value,
    provider_block: &str,
) -> anyhow::Result<()> {
    let catalog_path = home.join("model-catalog.json");
    std::fs::write(&catalog_path, serde_json::to_string_pretty(catalog)?)?;

    let catalog_abs = catalog_path.to_string_lossy();
    let config_content = format!(
        r#"
model_provider = "{provider_id}"
model = "{model}"
model_catalog_json = "{catalog_abs}"
approval_policy = "never"
sandbox_mode = "read-only"
web_search = "disabled"

[tools]
web_search = false

[features]
web_search_request = false

{provider_block}
"#,
        provider_id = provider_id,
        model = model,
        catalog_abs = catalog_abs,
        provider_block = provider_block.trim(),
    );
    std::fs::write(home.join("config.toml"), config_content.trim())?;
    Ok(())
}

fn write_codex_home_dir_default_provider(
    home: &Path,
    model: &str,
    catalog: &serde_json::Value,
) -> anyhow::Result<()> {
    let catalog_path = home.join("model-catalog.json");
    std::fs::write(&catalog_path, serde_json::to_string_pretty(catalog)?)?;

    let catalog_abs = catalog_path.to_string_lossy();
    let config_content = format!(
        r#"
model = "{model}"
model_catalog_json = "{catalog_abs}"
approval_policy = "never"
sandbox_mode = "read-only"
web_search = "disabled"

[tools]
web_search = false

[features]
web_search_request = false
"#,
        model = model,
        catalog_abs = catalog_abs,
    );
    std::fs::write(home.join("config.toml"), config_content.trim())?;
    Ok(())
}

fn write_codex_home_project_trust_only(home: &Path, project_dir: &Path) -> anyhow::Result<()> {
    let project_dir = project_dir.to_string_lossy();
    let config_content = format!(
        r#"
approval_policy = "never"
sandbox_mode = "read-only"
web_search = "disabled"

[tools]
web_search = false

[features]
web_search_request = false

[projects."{project_dir}"]
trust_level = "trusted"
"#
    );
    std::fs::write(home.join("config.toml"), config_content.trim())?;
    Ok(())
}

fn local_shim_provider_block(shim_url: &str, stream_timeout_ms: u32) -> String {
    format!(
        r#"[model_providers.local-shim]
name = "codex-shim"
base_url = "{shim_url}"
env_key = "LOCAL_SHIM_TOKEN"
wire_api = "responses"
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
stream_idle_timeout_ms = {stream_timeout_ms}
"#
    )
}

pub fn generate_codex_home_with_provider(
    base: &Path,
    provider_id: &str,
    provider_block: &str,
    model: &str,
    catalog: &serde_json::Value,
) -> anyhow::Result<PathBuf> {
    let home = base.join("codex_home");
    std::fs::create_dir_all(&home)?;
    write_codex_home_dir(&home, provider_id, model, catalog, provider_block)?;
    Ok(home)
}

pub fn generate_codex_home_default_provider(
    base: &Path,
    model: &str,
    catalog: &serde_json::Value,
) -> anyhow::Result<PathBuf> {
    let home = base.join("codex_home");
    std::fs::create_dir_all(&home)?;
    write_codex_home_dir_default_provider(&home, model, catalog)?;
    Ok(home)
}

pub fn generate_codex_home_project_trust_only(
    base: &Path,
    project_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let home = base.join("codex_home");
    std::fs::create_dir_all(&home)?;
    write_codex_home_project_trust_only(&home, project_dir)?;
    Ok(home)
}

/// Generate CODEX_HOME with full reasoning support (for blackbox tests).
pub fn generate_codex_home(base: &Path, shim_url: &str, model: &str) -> anyhow::Result<PathBuf> {
    let catalog = build_catalog_json(model, true);
    generate_codex_home_with_provider(
        base,
        "local-shim",
        &local_shim_provider_block(shim_url, 120000),
        model,
        &catalog,
    )
}

/// Generate CODEX_HOME with reasoning disabled (for live smoke tests).
pub fn generate_codex_home_bare(
    base: &Path,
    shim_url: &str,
    model: &str,
) -> anyhow::Result<PathBuf> {
    let catalog = build_catalog_json(model, false);
    generate_codex_home_with_provider(
        base,
        "local-shim",
        &local_shim_provider_block(shim_url, 300000),
        model,
        &catalog,
    )
}

pub fn write_project_codex_config(
    project_dir: &Path,
    shim_url: &str,
    model: &str,
) -> anyhow::Result<()> {
    let catalog = build_catalog_json(model, true);
    let codex_dir = project_dir.join(".codex");
    std::fs::create_dir_all(codex_dir.join("codex-shim"))?;
    let catalog_path = codex_dir.join("codex-shim").join("model-catalog.json");
    std::fs::write(&catalog_path, serde_json::to_string_pretty(&catalog)?)?;

    let catalog_abs = catalog_path.to_string_lossy();
    let config_content = format!(
        r#"
model_provider = "local-shim"
model = "{model}"
model_catalog_json = "{catalog_abs}"
approval_policy = "never"
sandbox_mode = "read-only"
web_search = "disabled"

[tools]
web_search = false

[features]
web_search_request = false

{provider_block}
"#,
        model = model,
        catalog_abs = catalog_abs,
        provider_block = local_shim_provider_block(shim_url, 120000).trim(),
    );
    std::fs::write(codex_dir.join("config.toml"), config_content.trim())?;
    Ok(())
}

pub fn discover_codex_auth_json() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("CODEX_SHIM_E2E_OPENAI_AUTH_JSON") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        let candidate = PathBuf::from(codex_home).join("auth.json");
        if candidate.exists() {
            return Some(candidate);
        }
    }

    home_dir()
        .map(|home| home.join(".codex").join("auth.json"))
        .filter(|candidate| candidate.exists())
}

pub fn seed_codex_home_auth(home: &Path, auth_json_path: &Path) -> anyhow::Result<()> {
    if !auth_json_path.exists() {
        anyhow::bail!("auth.json not found: {}", auth_json_path.display());
    }
    std::fs::copy(auth_json_path, home.join("auth.json"))?;
    Ok(())
}

pub fn create_workspace_tempdir(prefix: &str) -> anyhow::Result<tempfile::TempDir> {
    let ws_root = find_workspace_root()?;
    let base = ws_root.join("target").join("e2e-live");
    std::fs::create_dir_all(&base)?;
    Ok(tempfile::Builder::new().prefix(prefix).tempdir_in(base)?)
}

// ── Temp Shim Config ─────────────────────────────────────────────

pub fn generate_shim_config(
    base: &Path,
    upstream_url: &str,
    provider_kind: &str,
    model: &str,
) -> anyhow::Result<PathBuf> {
    let config_path = base.join("shim_config.yaml");
    let content = format!(
        r#"
server:
  listen: "127.0.0.1:__PORT__"
  base_path: "/v1"
  auth:
    mode: optional-bearer
    accepted_bearer_tokens:
      - "local-shim-test-token"

upstream:
  base_url: "{upstream_url}"
  chat_path: "/chat/completions"
  responses_path: "/responses"
  models_path: "/models"
  api_key_env: "UPSTREAM_MOCK_API_KEY"
  http_headers:
    X-E2E-Static: "static-value"
  env_http_headers:
    X-E2E-From-Env: "E2E_DYNAMIC_HEADER"
  query_params:
    api-version: "e2e-test"
  timeout_seconds: 30
  connect_timeout_seconds: 10
  max_retries: 0

provider:
  kind: {provider_kind}

models:
  default: "{model}"
  map:
    codex-default: "{model}"
  catalog:
    - slug: "{model}"
      display_name: "{model}"
      context_window: 131072
      tool_calling: true
      vision: false

state:
  backend: memory
  ttl_seconds: 3600

logging:
  level: debug
  redact_api_keys: true
  redact_message_content: false
"#,
        upstream_url = upstream_url,
        provider_kind = provider_kind,
        model = model,
    );
    std::fs::write(&config_path, content.trim())?;
    Ok(config_path)
}

pub fn generate_shim_config_native(
    base: &Path,
    upstream_url: &str,
    provider_kind: &str,
    model: &str,
) -> anyhow::Result<PathBuf> {
    let config_path = base.join("shim_config_native.yaml");
    let content = format!(
        r#"
server:
  listen: "127.0.0.1:__PORT__"
  base_path: "/v1"
  auth:
    mode: optional-bearer
    accepted_bearer_tokens:
      - "local-shim-test-token"

upstream:
  base_url: "{upstream_url}"
  chat_path: "/chat/completions"
  responses_path: "/responses"
  models_path: "/models"
  api_key_env: "UPSTREAM_MOCK_API_KEY"
  timeout_seconds: 30
  connect_timeout_seconds: 10
  max_retries: 0

provider:
  kind: {provider_kind}

models:
  default: "{model}"
  map:
    codex-default: "{model}"
  catalog:
    - slug: "{model}"
      display_name: "{model}"
      context_window: 131072
      tool_calling: true
      vision: false

state:
  backend: memory
  ttl_seconds: 3600

logging:
  level: debug
  redact_api_keys: true
  redact_message_content: false
"#,
        upstream_url = upstream_url,
        provider_kind = provider_kind,
        model = model,
    );
    std::fs::write(&config_path, content.trim())?;
    Ok(config_path)
}

// ── Shim binary discovery ────────────────────────────────────────

fn find_workspace_root() -> anyhow::Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        let cargo_toml = dir.join("Cargo.toml");
        if cargo_toml.exists() {
            if let Ok(content) = std::fs::read_to_string(&cargo_toml) {
                if content.contains("[workspace]") {
                    return Ok(dir);
                }
            }
        }
        if !dir.pop() {
            anyhow::bail!("workspace root not found (no Cargo.toml with [workspace])");
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn shim_binary_name() -> &'static str {
    if cfg!(windows) {
        "codex-shim.exe"
    } else {
        "codex-shim"
    }
}

fn resolve_built_shim_binary(ws_root: &Path) -> Option<PathBuf> {
    let candidates = [
        ws_root
            .join("target")
            .join("debug")
            .join(shim_binary_name()),
        ws_root
            .join("target")
            .join("release")
            .join(shim_binary_name()),
    ];
    candidates.into_iter().find(|candidate| candidate.exists())
}

fn find_or_build_shim_binary() -> anyhow::Result<PathBuf> {
    static SHIM_BINARY: OnceLock<PathBuf> = OnceLock::new();

    if let Some(path) = SHIM_BINARY.get() {
        return Ok(path.clone());
    }
    if let Ok(path) = std::env::var("CODEX_SHIM_BIN") {
        let p = PathBuf::from(&path);
        if !p.exists() {
            anyhow::bail!("CODEX_SHIM_BIN is set but does not exist: {}", p.display());
        }
        return Ok(p);
    }
    if let Ok(path) = std::env::var("CARGO_BIN_EXE_codex-shim") {
        let p = PathBuf::from(&path);
        if p.exists() {
            let _ = SHIM_BINARY.set(p.clone());
            return Ok(p);
        }
    }
    let ws_root = find_workspace_root()?;

    eprintln!(
        "e2e-codex: ensuring codex-shim is up to date via cargo build -p codex-shim\n\
         (set CODEX_SHIM_BIN to skip this step)"
    );
    let status = std::process::Command::new("cargo")
        .args(["build", "-p", "codex-shim"])
        .current_dir(&ws_root)
        .status()?;
    if !status.success() {
        anyhow::bail!(
            "cargo build -p codex-shim failed with exit code {:?}",
            status.code()
        );
    }
    if let Some(built) = resolve_built_shim_binary(&ws_root) {
        let _ = SHIM_BINARY.set(built.clone());
        return Ok(built);
    }
    for dir in std::env::split_paths(&std::env::var("PATH").unwrap_or_default()) {
        let candidate = dir.join(shim_binary_name());
        if candidate.exists() {
            let _ = SHIM_BINARY.set(candidate.clone());
            return Ok(candidate);
        }
    }
    anyhow::bail!(
        "cargo build -p codex-shim succeeded but binary not found.\n\
         Set CODEX_SHIM_BIN to the binary path."
    )
}

// ── Shim subprocess ──────────────────────────────────────────────

pub struct ShimProcess {
    child: tokio::process::Child,
    pub port: u16,
}

impl ShimProcess {
    pub async fn start(config_path: &Path) -> anyhow::Result<Self> {
        let shim_bin = find_or_build_shim_binary()?;
        start_with_bin(
            shim_bin,
            config_path,
            &[
                ("UPSTREAM_MOCK_API_KEY", "mock-api-key-for-testing"),
                ("E2E_DYNAMIC_HEADER", "dynamic-value"),
            ],
            "debug",
        )
        .await
    }

    pub async fn start_with_env(
        config_path: &Path,
        extra_env: &[(&str, &str)],
    ) -> anyhow::Result<Self> {
        let shim_bin = find_or_build_shim_binary()?;
        start_with_bin(shim_bin, config_path, extra_env, "info").await
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/v1", self.port)
    }
}

async fn start_with_bin(
    shim_bin: PathBuf,
    config_path: &Path,
    extra_env: &[(&str, &str)],
    log_level: &str,
) -> anyhow::Result<ShimProcess> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    drop(listener);

    let config_content = std::fs::read_to_string(config_path)?;
    let config_content = config_content.replace("__PORT__", &port.to_string());
    std::fs::write(config_path, &config_content)?;

    let mut cmd = tokio::process::Command::new(&shim_bin);
    cmd.arg("--config")
        .arg(config_path)
        .env("RUST_LOG", log_level)
        .kill_on_drop(true)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let child = cmd.spawn()?;

    let health_url = format!("http://127.0.0.1:{port}/healthz");
    for i in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        match reqwest::get(&health_url).await {
            Ok(resp) if resp.status().is_success() => {
                return Ok(ShimProcess { child, port });
            }
            _ => {
                if i == 49 {
                    anyhow::bail!("codex-shim failed to start within 10 seconds (port {port})");
                }
            }
        }
    }
    anyhow::bail!("codex-shim failed to start (port {port})")
}

impl Drop for ShimProcess {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

// ── Live Provider Types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ProviderSecretsFile {
    #[serde(flatten)]
    pub providers: std::collections::BTreeMap<String, ProviderCase>,
}

#[derive(Debug, Deserialize)]
pub struct ProviderCase {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub profile: String,
    pub model: String,
    pub base_url: String,
    #[serde(default)]
    pub chat_path: Option<String>,
    #[serde(default)]
    pub responses_path: Option<String>,
    pub api_key_env: String,
    pub api_key: String,
    #[serde(default)]
    pub query_params: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub http_headers: std::collections::HashMap<String, String>,
}

fn default_enabled() -> bool {
    false
}

pub fn read_provider_matrix(path: &str) -> anyhow::Result<ProviderSecretsFile> {
    let content = std::fs::read_to_string(path)?;
    let matrix: ProviderSecretsFile = toml::from_str(&content)?;
    Ok(matrix)
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_catalog_contains_required_codex_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let home =
            generate_codex_home_bare(tmp.path(), "http://127.0.0.1:8787/v1", "mock-model").unwrap();
        let catalog_path = home.join("model-catalog.json");
        let catalog: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(catalog_path).unwrap()).unwrap();

        let model = &catalog["models"][0];

        assert!(
            model.get("supported_reasoning_levels").is_some(),
            "bare catalog must have supported_reasoning_levels"
        );
        assert!(
            model["supported_reasoning_levels"].as_array().is_some(),
            "supported_reasoning_levels must be an array"
        );
        assert!(
            model.get("default_reasoning_level").is_some(),
            "bare catalog must have default_reasoning_level"
        );
        assert!(
            model.get("web_search_tool_type").is_none(),
            "web_search_tool_type must not be present"
        );
        assert_eq!(model["supports_search_tool"], false);
    }
}
