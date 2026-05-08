#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::VecDeque;
use std::fmt::{self, Debug};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use codex_shim::{
    app_with_metrics,
    codex_integration::{
        self, CodexIntegrationApplyResult, CodexIntegrationOptions, CodexIntegrationPreview,
        DesktopDoctorReport, ShimConfigSummary,
    },
    config::Config,
    provider_profile_config::ProviderProfileConfig,
    runtime_metrics::{RuntimeMetrics, RuntimeMetricsSnapshot, TokenSeriesSnapshot},
};
use protocol::provider_caps::EndpointMode;
use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::oneshot;
use tracing::{Event, Subscriber};
use tracing_subscriber::{
    Layer, Registry, fmt as tracing_fmt, layer::Context as LayerContext, layer::SubscriberExt,
    registry::LookupSpan, util::SubscriberInitExt,
};

#[derive(Clone, Default)]
struct SharedLogBuffer {
    inner: Arc<Mutex<LogBufferState>>,
}

#[derive(Default)]
struct LogBufferState {
    next_id: u64,
    entries: VecDeque<LogEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct LogEntry {
    id: u64,
    timestamp: String,
    level: String,
    message: String,
}

#[derive(Debug, Clone, Serialize)]
struct LogBatch {
    entries: Vec<LogEntry>,
    next_cursor: u64,
}

impl SharedLogBuffer {
    fn push(&self, level: String, message: String) {
        let mut state = self.inner.lock().expect("log buffer lock poisoned");
        let id = state.next_id;
        state.next_id += 1;
        state.entries.push_back(LogEntry {
            id,
            timestamp: chrono::Local::now().format("%H:%M:%S").to_string(),
            level,
            message,
        });
        while state.entries.len() > 2_000 {
            state.entries.pop_front();
        }
    }

    fn since(&self, cursor: Option<u64>) -> LogBatch {
        let state = self.inner.lock().expect("log buffer lock poisoned");
        let cursor = cursor.unwrap_or(0);
        let entries = state
            .entries
            .iter()
            .filter(|entry| entry.id >= cursor)
            .cloned()
            .collect::<Vec<_>>();
        LogBatch {
            entries,
            next_cursor: state.next_id,
        }
    }
}

struct GuiLogLayer {
    buffer: SharedLogBuffer,
}

impl<S> Layer<S> for GuiLogLayer
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: LayerContext<'_, S>) {
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);
        let level = event.metadata().level().to_string();
        let message = visitor.render();
        self.buffer.push(level, message);
    }
}

#[derive(Default)]
struct EventVisitor {
    message: Option<String>,
    fields: Vec<(String, String)>,
}

impl EventVisitor {
    fn render(self) -> String {
        match (self.message, self.fields.is_empty()) {
            (Some(message), true) => message,
            (Some(message), false) => {
                let extras = self
                    .fields
                    .into_iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("{message} {extras}")
            }
            (None, _) => self
                .fields
                .into_iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    fn record_value(&mut self, field: &tracing::field::Field, value: String) {
        if field.name() == "message" {
            self.message = Some(value);
        } else {
            self.fields.push((field.name().to_string(), value));
        }
    }
}

impl tracing::field::Visit for EventVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn Debug) {
        self.record_value(field, format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_value(field, value.to_string());
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_value(field, value.to_string());
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_value(field, value.to_string());
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_value(field, value.to_string());
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.record_value(field, value.to_string());
    }
}

#[derive(Default)]
struct RuntimeController {
    running: Option<RunningRuntime>,
}

struct RunningRuntime {
    shutdown: Option<oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<anyhow::Result<()>>,
    metrics: Arc<RuntimeMetrics>,
    summary: ShimConfigSummary,
    endpoint_mode: String,
}

struct GuiState {
    runtime: Mutex<RuntimeController>,
    logs: SharedLogBuffer,
}

impl Default for GuiState {
    fn default() -> Self {
        Self {
            runtime: Mutex::new(RuntimeController::default()),
            logs: SharedLogBuffer::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ConfigRequest {
    config_text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ShimDocumentRequest {
    path: String,
    config_text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct IntegrationRequest {
    config_text: String,
    #[serde(default)]
    config_path: Option<String>,
    options: CodexIntegrationOptions,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeStartRequest {
    config_text: String,
}

#[derive(Debug, Clone, Serialize)]
struct ShimInspection {
    summary: ShimConfigSummary,
    endpoint_mode: String,
    catalog_json: String,
}

#[derive(Debug, Clone, Serialize)]
struct ShimDocumentResponse {
    path: String,
    text: String,
    inspection: ShimInspection,
}

#[derive(Debug, Clone, Serialize)]
struct RuntimeView {
    running: bool,
    listen: String,
    provider: String,
    endpoint_mode: String,
    upstream_base_url: String,
    model: String,
    state_backend: String,
    uptime_seconds: u64,
    request_count: u64,
    completed_request_count: u64,
    error_count: u64,
    store_size: usize,
    last_error: Option<String>,
    token_series: TokenSeriesSnapshot,
}

#[derive(Debug, Clone, Serialize)]
struct DefaultsResponse {
    default_config_path: Option<String>,
    default_config_exists: bool,
    default_codex_home: Option<String>,
    current_directory: Option<String>,
    starter_config_text: String,
    browse_supported: bool,
    browse_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct BrowsePathRequest {
    kind: BrowsePathKind,
    #[serde(default)]
    initial_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BrowsePathKind {
    ConfigFile,
    Directory,
}

#[derive(Debug, Clone, Deserialize)]
struct ProbeRequest {
    base_url: String,
    #[serde(default)]
    api_key_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RuntimeSnapshotRequest {
    range_minutes: u32,
}

#[tauri::command]
fn get_defaults() -> Result<DefaultsResponse, String> {
    Ok(build_defaults_response(
        home_dir(),
        std::env::current_dir().ok(),
    ))
}

#[tauri::command]
fn browse_path(request: BrowsePathRequest) -> Result<Option<String>, String> {
    if let Some(message) = browse_unavailable_message() {
        return Err(message);
    }
    let selected = match request.kind {
        BrowsePathKind::ConfigFile => browse_config_file(request.initial_path.as_deref()),
        BrowsePathKind::Directory => browse_directory(request.initial_path.as_deref()),
    };
    Ok(selected.map(|path| path.display().to_string()))
}

#[tauri::command]
fn inspect_shim_config(request: ConfigRequest) -> Result<ShimInspection, String> {
    inspect_config_text(&request.config_text).map_err(stringify_error)
}

#[tauri::command]
fn load_shim_config(path: String) -> Result<ShimDocumentResponse, String> {
    let path = PathBuf::from(path);
    let text = codex_integration::load_config_text(&path).map_err(stringify_error)?;
    let inspection = inspect_config_text(&text).map_err(stringify_error)?;
    Ok(ShimDocumentResponse {
        path: path.display().to_string(),
        text,
        inspection,
    })
}

#[tauri::command]
fn save_shim_config(request: ShimDocumentRequest) -> Result<ShimDocumentResponse, String> {
    let path = PathBuf::from(&request.path);
    inspect_config_text(&request.config_text).map_err(stringify_error)?;
    codex_integration::save_config_text(&path, &request.config_text).map_err(stringify_error)?;
    let inspection = inspect_config_text(&request.config_text).map_err(stringify_error)?;
    Ok(ShimDocumentResponse {
        path: path.display().to_string(),
        text: request.config_text,
        inspection,
    })
}

#[tauri::command]
fn preview_codex_integration(
    request: IntegrationRequest,
) -> Result<CodexIntegrationPreview, String> {
    let config =
        codex_integration::parse_config_text(&request.config_text).map_err(stringify_error)?;
    codex_integration::preview_codex_integration(&config, &request.options).map_err(stringify_error)
}

#[tauri::command]
fn apply_codex_integration(
    request: IntegrationRequest,
) -> Result<CodexIntegrationApplyResult, String> {
    let config_path = request.config_path.as_deref().map(Path::new);
    codex_integration::apply_codex_integration(&request.config_text, config_path, &request.options)
        .map_err(stringify_error)
}

#[tauri::command]
fn doctor_desktop(request: IntegrationRequest) -> Result<DesktopDoctorReport, String> {
    let config =
        codex_integration::parse_config_text(&request.config_text).map_err(stringify_error)?;
    codex_integration::doctor_desktop(&config, &request.options).map_err(stringify_error)
}

#[tauri::command]
async fn probe_upstream(request: ProbeRequest) -> Result<serde_json::Value, String> {
    run_probe(&request.base_url, request.api_key_env.as_deref())
        .await
        .map_err(stringify_error)
}

#[tauri::command]
async fn start_runtime(
    state: State<'_, GuiState>,
    request: RuntimeStartRequest,
) -> Result<RuntimeView, String> {
    start_runtime_impl(state.inner(), request).await
}

async fn start_runtime_impl(
    gui_state: &GuiState,
    request: RuntimeStartRequest,
) -> Result<RuntimeView, String> {
    let config =
        codex_integration::parse_config_text(&request.config_text).map_err(stringify_error)?;
    let summary = codex_integration::config_summary(&config);
    let endpoint_mode = endpoint_mode_label(&config);
    let listen = config.server.listen.clone();

    if let Some(existing) = take_running_runtime(gui_state) {
        stop_running(existing).await.map_err(stringify_error)?;
    }

    let metrics = Arc::new(RuntimeMetrics::default());
    let app = app_with_metrics(config, metrics.clone()).map_err(stringify_error)?;
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .map_err(stringify_error)?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
            .map_err(anyhow::Error::from)
    });

    let running = RunningRuntime {
        shutdown: Some(shutdown_tx),
        join,
        metrics,
        summary,
        endpoint_mode,
    };
    {
        let mut guard = gui_state.runtime.lock().expect("runtime lock poisoned");
        guard.running = Some(running);
    }
    get_runtime_snapshot_impl(gui_state, RuntimeSnapshotRequest { range_minutes: 60 }).await
}

#[tauri::command]
async fn restart_runtime(
    state: State<'_, GuiState>,
    request: RuntimeStartRequest,
) -> Result<RuntimeView, String> {
    start_runtime_impl(state.inner(), request).await
}

#[tauri::command]
async fn stop_runtime(state: State<'_, GuiState>) -> Result<RuntimeView, String> {
    stop_runtime_impl(state.inner()).await
}

async fn stop_runtime_impl(gui_state: &GuiState) -> Result<RuntimeView, String> {
    if let Some(existing) = take_running_runtime(gui_state) {
        stop_running(existing).await.map_err(stringify_error)?;
    }
    get_runtime_snapshot_impl(gui_state, RuntimeSnapshotRequest { range_minutes: 60 }).await
}

#[tauri::command]
async fn get_runtime_snapshot(
    state: State<'_, GuiState>,
    request: RuntimeSnapshotRequest,
) -> Result<RuntimeView, String> {
    get_runtime_snapshot_impl(state.inner(), request).await
}

async fn get_runtime_snapshot_impl(
    gui_state: &GuiState,
    request: RuntimeSnapshotRequest,
) -> Result<RuntimeView, String> {
    let metrics = RuntimeMetrics::default();
    let empty_series = metrics.token_series(normalize_range(request.range_minutes));
    let guard = gui_state.runtime.lock().expect("runtime lock poisoned");
    if let Some(running) = &guard.running {
        let snapshot = running.metrics.snapshot();
        Ok(build_runtime_view(
            true,
            &running.summary,
            &running.endpoint_mode,
            snapshot,
            running
                .metrics
                .token_series(normalize_range(request.range_minutes)),
        ))
    } else {
        Ok(RuntimeView {
            running: false,
            listen: String::new(),
            provider: String::new(),
            endpoint_mode: String::new(),
            upstream_base_url: String::new(),
            model: String::new(),
            state_backend: String::new(),
            uptime_seconds: 0,
            request_count: 0,
            completed_request_count: 0,
            error_count: 0,
            store_size: 0,
            last_error: None,
            token_series: empty_series,
        })
    }
}

#[tauri::command]
fn get_logs(state: State<'_, GuiState>, cursor: Option<u64>) -> Result<LogBatch, String> {
    Ok(get_logs_impl(state.inner(), cursor))
}

fn get_logs_impl(gui_state: &GuiState, cursor: Option<u64>) -> LogBatch {
    gui_state.logs.since(cursor)
}

fn build_runtime_view(
    running: bool,
    summary: &ShimConfigSummary,
    endpoint_mode: &str,
    metrics: RuntimeMetricsSnapshot,
    token_series: TokenSeriesSnapshot,
) -> RuntimeView {
    RuntimeView {
        running,
        listen: summary.listen.clone(),
        provider: summary.provider_kind.clone(),
        endpoint_mode: endpoint_mode.to_string(),
        upstream_base_url: summary.upstream_base_url.clone(),
        model: summary.model.clone(),
        state_backend: summary.state_backend.clone(),
        uptime_seconds: metrics.uptime_seconds,
        request_count: metrics.request_count,
        completed_request_count: metrics.completed_request_count,
        error_count: metrics.error_count,
        store_size: metrics.store_size,
        last_error: metrics.last_error,
        token_series,
    }
}

fn inspect_config_text(config_text: &str) -> anyhow::Result<ShimInspection> {
    let config = codex_integration::parse_config_text(config_text)?;
    Ok(ShimInspection {
        summary: codex_integration::config_summary(&config),
        endpoint_mode: endpoint_mode_label(&config),
        catalog_json: codex_integration::render_model_catalog_json(&config, None)?,
    })
}

fn endpoint_mode_label(config: &Config) -> String {
    let profile_cfg =
        config
            .provider
            .profile_config
            .clone()
            .unwrap_or_else(|| ProviderProfileConfig {
                profile: config.provider.kind.clone(),
                ..Default::default()
            });
    match profile_cfg.build_profile().capabilities().endpoint_mode {
        EndpointMode::ChatCompletionsShim => "chat_shim".into(),
        EndpointMode::NativeResponses => "native_responses".into(),
        EndpointMode::StatelessResponses => "stateless_responses".into(),
    }
}

fn take_running_runtime(gui_state: &GuiState) -> Option<RunningRuntime> {
    let mut guard = gui_state.runtime.lock().expect("runtime lock poisoned");
    guard.running.take()
}

async fn stop_running(mut running: RunningRuntime) -> anyhow::Result<()> {
    if let Some(shutdown) = running.shutdown.take() {
        let _ = shutdown.send(());
    }
    running
        .join
        .await
        .map_err(|err| anyhow!("runtime join error: {err}"))??;
    Ok(())
}

fn normalize_range(range_minutes: u32) -> u32 {
    match range_minutes {
        15 | 60 | 1440 => range_minutes,
        _ => 60,
    }
}

async fn run_probe(base_url: &str, api_key_env: Option<&str>) -> anyhow::Result<serde_json::Value> {
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

    let chat_url = format!("{base_url}/chat/completions");
    let mut req = client.post(&chat_url).json(&serde_json::json!({
        "model": "probe",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1,
        "stream": false,
    }));
    if let Some(key_env) = api_key_env
        && let Ok(key) = std::env::var(key_env)
    {
        req = req.bearer_auth(key);
    }
    match req.send().await {
        Ok(resp) => {
            result["chat_completions"] = serde_json::Value::Bool(true);
            if let Ok(body) = resp.text().await
                && let Ok(json) = serde_json::from_str::<serde_json::Value>(&body)
            {
                if json.get("choices").is_some() {
                    result["chat_completions"] = serde_json::Value::Bool(true);
                }
                if json.get("usage").is_some() {
                    result["usage_in_stream_final"] = serde_json::Value::Bool(true);
                }
            }
        }
        Err(e) => {
            result["errors"] = serde_json::json!([format!("chat_completions: {e}")]);
        }
    }

    let resp_url = format!("{base_url}/responses");
    let mut req = client.post(&resp_url).json(&serde_json::json!({
        "model": "probe",
        "input": "hi",
        "max_output_tokens": 1,
    }));
    if let Some(key_env) = api_key_env
        && let Ok(key) = std::env::var(key_env)
    {
        req = req.bearer_auth(key);
    }
    match req.send().await {
        Ok(resp) => {
            if let Ok(body) = resp.text().await
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
        Err(e) => append_error(&mut result, format!("responses: {e}")),
    }

    let models_url = format!("{base_url}/models");
    let mut req = client.get(&models_url);
    if let Some(key_env) = api_key_env
        && let Ok(key) = std::env::var(key_env)
    {
        req = req.bearer_auth(key);
    }
    if req.send().await.is_ok() {
        result["models"] = serde_json::Value::Bool(true);
    }

    Ok(result)
}

fn build_defaults_response(
    home: Option<PathBuf>,
    current_directory: Option<PathBuf>,
) -> DefaultsResponse {
    let default_config_path = home
        .as_ref()
        .map(|home| home.join(".codex-shim").join("config.yaml"));
    let default_config_exists = default_config_path
        .as_ref()
        .is_some_and(|path| path.exists());
    let browse_message = browse_unavailable_message();
    DefaultsResponse {
        default_config_path: default_config_path.map(|path| path.display().to_string()),
        default_config_exists,
        default_codex_home: home.map(|home| home.join(".codex").display().to_string()),
        current_directory: current_directory.map(|path| path.display().to_string()),
        starter_config_text: starter_config_text().to_string(),
        browse_supported: browse_message.is_none(),
        browse_message,
    }
}

fn starter_config_text() -> &'static str {
    include_str!("../../../examples/generic-openai-chat/config.yaml")
}

fn browse_config_file(initial_path: Option<&str>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().add_filter("YAML", &["yaml", "yml"]);
    if let Some(path) = initial_path.and_then(non_empty_path).map(PathBuf::from) {
        if let Some(parent) = path.parent().and_then(best_existing_directory) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
            dialog = dialog.set_file_name(file_name);
        }
    } else if let Some(home) = home_dir() {
        let shim_dir = home.join(".codex-shim");
        let directory = best_existing_directory(shim_dir.as_path()).unwrap_or(home);
        dialog = dialog.set_directory(directory);
    }
    dialog.pick_file()
}

fn browse_directory(initial_path: Option<&str>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new();
    if let Some(path) = initial_path.and_then(non_empty_path).map(PathBuf::from) {
        let directory = if path.is_dir() {
            path
        } else {
            path.parent().unwrap_or(path.as_path()).to_path_buf()
        };
        if let Some(directory) = best_existing_directory(&directory) {
            dialog = dialog.set_directory(directory);
        }
    }
    dialog.pick_folder()
}

fn non_empty_path(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn best_existing_directory(path: &Path) -> Option<PathBuf> {
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        if current.exists() && current.is_dir() {
            return Some(current.to_path_buf());
        }
        candidate = current.parent();
    }
    None
}

fn browse_unavailable_message() -> Option<String> {
    if is_wsl_environment() {
        Some(
            "Native Browse dialogs are disabled under WSL because they can hang the GUI. Paste the path manually instead.".into(),
        )
    } else {
        None
    }
}

fn is_wsl_environment() -> bool {
    if std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some() {
        return true;
    }
    if cfg!(target_os = "linux")
        && let Ok(version) = std::fs::read_to_string("/proc/version")
    {
        let lower = version.to_ascii_lowercase();
        return lower.contains("microsoft") || lower.contains("wsl");
    }
    false
}

fn append_error(result: &mut serde_json::Value, value: String) {
    let mut errors = result["errors"].as_array().cloned().unwrap_or_default();
    errors.push(serde_json::json!(value));
    result["errors"] = serde_json::Value::Array(errors);
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn stringify_error(err: impl fmt::Display) -> String {
    err.to_string()
}

fn init_tracing(logs: SharedLogBuffer) {
    let _ = Registry::default()
        .with(tracing_fmt::layer().with_ansi(false))
        .with(GuiLogLayer { buffer: logs })
        .try_init();
}

fn main() {
    let state = GuiState::default();
    init_tracing(state.logs.clone());

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_defaults,
            browse_path,
            inspect_shim_config,
            load_shim_config,
            save_shim_config,
            preview_codex_integration,
            apply_codex_integration,
            doctor_desktop,
            probe_upstream,
            start_runtime,
            restart_runtime,
            stop_runtime,
            get_runtime_snapshot,
            get_logs
        ])
        .run(tauri::generate_context!())
        .expect("failed to run codex-shim GUI");
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use std::sync::Once;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::{
        Router,
        routing::{get, post},
    };
    use serde_json::json;

    static TRACING_INIT: Once = Once::new();

    fn ensure_test_tracing(logs: SharedLogBuffer) {
        TRACING_INIT.call_once(|| {
            init_tracing(logs);
        });
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock drift")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "codex-shim-gui-{label}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn defaults_report_missing_config_and_starter_template() {
        let home = unique_temp_dir("defaults-home");
        let cwd = unique_temp_dir("defaults-cwd");
        let defaults = build_defaults_response(Some(home.clone()), Some(cwd.clone()));
        let expected_config = home.join(".codex-shim").join("config.yaml");
        let expected_codex_home = home.join(".codex");
        let expected_config_display = expected_config.display().to_string();
        let expected_codex_home_display = expected_codex_home.display().to_string();
        let expected_cwd = cwd.display().to_string();

        assert_eq!(
            defaults.default_config_path.as_deref(),
            Some(expected_config_display.as_str())
        );
        assert!(!defaults.default_config_exists);
        assert_eq!(
            defaults.default_codex_home.as_deref(),
            Some(expected_codex_home_display.as_str())
        );
        assert_eq!(
            defaults.current_directory.as_deref(),
            Some(expected_cwd.as_str())
        );
        assert!(
            defaults
                .starter_config_text
                .contains("Generic OpenAI-compatible Chat")
        );
        assert_eq!(
            defaults.browse_supported,
            browse_unavailable_message().is_none()
        );
        assert_eq!(defaults.browse_message, browse_unavailable_message());
    }

    fn smoke_config(listen: &str, upstream_base_url: &str) -> String {
        format!(
            r#"
server:
  listen: "{listen}"
  base_path: "/v1"

upstream:
  base_url: "{upstream_base_url}"
  chat_path: "/chat/completions"
  models_path: "/models"
  timeout_seconds: 30
  connect_timeout_seconds: 5
  max_retries: 0

provider:
  kind: generic-chat
  profile_config:
    profile: generic-chat

reasoning:
  enabled: false
  effort: high

models:
  default: "mock-model"
  map:
    codex-default: "mock-model"
  catalog:
    - slug: "mock-model"
      display_name: "Mock Model"
      context_window: 131072

state:
  backend: memory
  ttl_seconds: 86400
  cleanup_interval_seconds: 3600

logging:
  level: info
  redact_api_keys: true
"#
        )
    }

    async fn spawn_mock_chat_upstream() -> (String, tokio::task::JoinHandle<()>) {
        async fn chat_handler() -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "id": "chatcmpl_mock_1",
                "object": "chat.completion",
                "created": 1714771200,
                "model": "mock-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "mock ok"},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 11,
                    "completion_tokens": 7,
                    "total_tokens": 18
                }
            }))
        }

        async fn models_handler() -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "object": "list",
                "data": [{
                    "id": "mock-model",
                    "object": "model",
                    "owned_by": "tests"
                }]
            }))
        }

        let router = Router::new()
            .route("/v1/chat/completions", post(chat_handler))
            .route("/v1/models", get(models_handler));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock upstream");
        let addr = listener.local_addr().expect("local addr");
        let join = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve mock upstream");
        });
        (format!("http://{addr}/v1"), join)
    }

    async fn find_free_listen() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind free port");
        let addr = listener.local_addr().expect("local addr");
        drop(listener);
        addr.to_string()
    }

    #[tokio::test]
    async fn gui_smoke_project_apply_and_doctor_roundtrip() {
        let project_dir = unique_temp_dir("project");
        let codex_home = unique_temp_dir("codex-home");
        let target_path = project_dir.join(".codex").join("config.toml");
        std::fs::create_dir_all(target_path.parent().expect("target parent")).expect("mkdir");
        std::fs::write(
            &target_path,
            "theme = \"classic\"\n\n[model_providers.other]\nname = \"Other\"\nbase_url = \"http://example.test\"\n",
        )
        .expect("seed project toml");

        let request = IntegrationRequest {
            config_text: smoke_config("127.0.0.1:9787", "http://127.0.0.1:8999/v1"),
            config_path: None,
            options: CodexIntegrationOptions {
                provider_id: "codex_shim".into(),
                model: None,
                profile: None,
                codex_home: Some(codex_home.display().to_string()),
                project_dir: Some(project_dir.display().to_string()),
                trust_project: true,
                env_key: None,
                web_search: None,
                base_url: None,
                base_toml_override: None,
            },
        };

        let preview = preview_codex_integration(request.clone()).expect("preview");
        assert_eq!(preview.target_path, target_path.display().to_string());
        assert!(preview.merged_toml.contains("theme = \"classic\""));
        assert!(preview.merged_toml.contains("[model_providers.other]"));
        assert!(
            preview
                .merged_toml
                .contains("model_provider = \"codex_shim\"")
        );
        assert!(preview.merged_toml.contains("model = \"mock-model\""));

        let apply = apply_codex_integration(request.clone()).expect("apply");
        assert_eq!(apply.target_path, target_path.display().to_string());
        assert!(
            project_dir
                .join(".codex")
                .join("model-catalog-shim.json")
                .exists()
        );

        let report = doctor_desktop(request).expect("doctor");
        assert!(
            !report.has_unsupported(),
            "doctor unexpectedly reported unsupported checks: {:?}",
            report.checks
        );
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.status == codex_integration::DesktopCheckStatus::Gated),
            "expected at least one gated desktop-resume caveat"
        );

        let global_toml =
            std::fs::read_to_string(codex_home.join("config.toml")).expect("read trust");
        assert!(global_toml.contains("trust_level = \"trusted\""));
        assert!(global_toml.contains(project_dir.display().to_string().as_str()));
    }

    #[tokio::test]
    async fn gui_smoke_global_apply_roundtrip() {
        let codex_home = unique_temp_dir("global-codex-home");
        let target_path = codex_home.join("config.toml");
        std::fs::write(
            &target_path,
            "theme = \"classic\"\n\n[model_providers.other]\nname = \"Other\"\nbase_url = \"http://example.test\"\n",
        )
        .expect("seed global toml");

        let request = IntegrationRequest {
            config_text: smoke_config("127.0.0.1:9788", "http://127.0.0.1:8999/v1"),
            config_path: None,
            options: CodexIntegrationOptions {
                provider_id: "codex_shim".into(),
                model: None,
                profile: None,
                codex_home: Some(codex_home.display().to_string()),
                project_dir: None,
                trust_project: false,
                env_key: None,
                web_search: None,
                base_url: None,
                base_toml_override: None,
            },
        };

        let preview = preview_codex_integration(request.clone()).expect("preview");
        assert_eq!(preview.mode, "global");
        assert_eq!(preview.target_path, target_path.display().to_string());
        assert!(preview.merged_toml.contains("theme = \"classic\""));
        assert!(preview.merged_toml.contains("[model_providers.other]"));
        assert!(preview.trust_target_path.is_none());

        let apply = apply_codex_integration(request).expect("apply");
        assert_eq!(apply.target_path, target_path.display().to_string());
        assert!(codex_home.join("model-catalog-shim.json").exists());

        let global_toml = std::fs::read_to_string(target_path).expect("read global toml");
        assert!(global_toml.contains("theme = \"classic\""));
        assert!(global_toml.contains("[model_providers.other]"));
        assert!(global_toml.contains("model_provider = \"codex_shim\""));
    }

    #[tokio::test]
    async fn gui_smoke_runtime_collects_usage_and_logs() {
        let (upstream_base_url, upstream_join) = spawn_mock_chat_upstream().await;
        let listen = find_free_listen().await;
        let gui_state = GuiState::default();
        ensure_test_tracing(gui_state.logs.clone());
        let config_text = smoke_config(&listen, &upstream_base_url);

        let started = start_runtime_impl(
            &gui_state,
            RuntimeStartRequest {
                config_text: config_text.clone(),
            },
        )
        .await
        .expect("start runtime");
        assert!(started.running);
        assert_eq!(started.listen, listen);

        let client = reqwest::Client::new();
        let response = client
            .post(format!("http://{listen}/v1/responses"))
            .json(&json!({
                "model": "mock-model",
                "input": "hello from gui smoke",
                "max_output_tokens": 32
            }))
            .send()
            .await
            .expect("send shim request");
        assert!(response.status().is_success(), "unexpected response status");
        let body = response.text().await.expect("response body");
        assert!(body.contains("mock ok"), "unexpected shim body: {body}");

        let snapshot =
            get_runtime_snapshot_impl(&gui_state, RuntimeSnapshotRequest { range_minutes: 60 })
                .await
                .expect("runtime snapshot");
        assert!(snapshot.running);
        assert_eq!(snapshot.request_count, 1);
        assert_eq!(snapshot.completed_request_count, 1);
        assert_eq!(snapshot.error_count, 0);
        assert!(
            snapshot
                .token_series
                .buckets
                .iter()
                .any(|bucket| bucket.total_tokens == 18
                    && bucket.input_tokens == 11
                    && bucket.output_tokens == 7),
            "token series did not record expected usage: {:?}",
            snapshot.token_series.buckets
        );

        let logs = get_logs_impl(&gui_state, None);
        assert!(
            !logs.entries.is_empty(),
            "expected GUI log buffer to capture runtime activity"
        );

        let stopped = stop_runtime_impl(&gui_state).await.expect("stop runtime");
        assert!(!stopped.running);

        upstream_join.abort();
    }
}
