// ── Live Provider Smoke E2E Tests ────────────────────────────────
//
// Run:  CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
//       cargo test -p e2e-codex --test codex_live -- --ignored --nocapture
//
// These tests require real provider API keys and network access.
// All tests are #[ignore] by default.

use e2e_codex::{
    CodexExecOptions, ProviderCase, ShimProcess, create_workspace_tempdir,
    discover_codex_auth_json, generate_codex_home_bare, generate_codex_home_default_provider,
    generate_codex_home_with_provider, read_provider_matrix, run_codex_exec,
    run_codex_exec_with_options, seed_codex_home_auth,
};
use protocol::models::{CatalogModelSpec, build_model_catalog};
use protocol::provider_caps::ProviderCapabilities;
use std::path::PathBuf;

// ── Helpers ──────────────────────────────────────────────────────

/// Return the last `n` lines of a string.
fn tail_lines(s: &str, n: usize) -> String {
    let lines: Vec<_> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn should_skip_live_tool_tests() -> bool {
    std::env::var("CODEX_SHIM_E2E_SKIP_TOOL_SMOKE")
        .ok()
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

#[derive(Debug, Clone)]
struct DifferentialScenario {
    name: &'static str,
    initial_files: &'static [(&'static str, &'static str)],
    prompts: &'static [&'static str],
    expected_turn_messages: &'static [&'static str],
    require_exact_turn_messages: bool,
    expected_files: &'static [(&'static str, &'static str)],
    sandbox: &'static str,
    require_tool_evidence: bool,
    require_compaction_evidence: bool,
    context_window: i64,
    auto_compact_token_limit: Option<i64>,
    json_output: bool,
    compare_final_messages: bool,
}

fn build_live_catalog_json(
    model: &str,
    context_window: i64,
    auto_compact_token_limit: Option<i64>,
) -> serde_json::Value {
    build_live_catalog_json_with_base_instructions(
        model,
        context_window,
        auto_compact_token_limit,
        "",
    )
}

fn build_live_catalog_json_with_base_instructions(
    model: &str,
    context_window: i64,
    auto_compact_token_limit: Option<i64>,
    base_instructions: &str,
) -> serde_json::Value {
    let caps = ProviderCapabilities {
        supports_function_tools: true,
        supports_parallel_tool_calls: true,
        supports_reasoning_effort: false,
        ..Default::default()
    };
    serde_json::to_value(build_model_catalog(
        &[CatalogModelSpec {
            slug: model.to_string(),
            display_name: Some(model.to_string()),
            description: None,
            context_window,
            tool_calling: Some(true),
            vision: Some(false),
            reasoning_levels: Some(vec![]),
            priority: Some(10),
            base_instructions: Some(base_instructions.to_string()),
            auto_compact_token_limit,
            supports_search_tool: Some(false),
            supports_reasoning_summaries: Some(false),
            apply_patch_tool_type: None,
            supports_image_detail_original: Some(false),
        }],
        &caps,
    ))
    .unwrap()
}

const OPENAI_AUTH_BASE_INSTRUCTIONS: &str = "You are Codex, a coding agent.";

#[derive(Debug, Clone)]
enum OpenAiBaselineAuth {
    ApiKey,
    ChatGptAuth { auth_json_path: PathBuf },
}

impl OpenAiBaselineAuth {
    fn label(&self) -> &'static str {
        match self {
            Self::ApiKey => "baseline-openai-direct",
            Self::ChatGptAuth { .. } => "baseline-openai-auth",
        }
    }
}

fn resolve_openai_baseline_auth() -> anyhow::Result<OpenAiBaselineAuth> {
    if std::env::var("OPENAI_API_KEY")
        .ok()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false)
    {
        return Ok(OpenAiBaselineAuth::ApiKey);
    }

    let auth_json_path = discover_codex_auth_json().ok_or_else(|| {
        anyhow::anyhow!(
            "set OPENAI_API_KEY (preferred) or provide a Codex auth cache via \
             CODEX_SHIM_E2E_OPENAI_AUTH_JSON/current CODEX_HOME/auth.json"
        )
    })?;

    Ok(OpenAiBaselineAuth::ChatGptAuth { auth_json_path })
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

fn openai_direct_provider_block(stream_timeout_ms: u32) -> String {
    format!(
        r#"[model_providers.openai-direct]
name = "OpenAI Direct"
base_url = "https://api.openai.com/v1"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
stream_idle_timeout_ms = {stream_timeout_ms}
"#
    )
}

fn create_fixture_dir(
    root: &std::path::Path,
    files: &[(&str, &str)],
) -> anyhow::Result<std::path::PathBuf> {
    let fixture = root.join("fixture");
    std::fs::create_dir_all(&fixture)?;
    for (rel, content) in files {
        let path = fixture.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    Ok(fixture)
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn read_expected_files(
    workdir: &std::path::Path,
    files: &[(&str, &str)],
) -> anyhow::Result<Vec<(String, String)>> {
    files
        .iter()
        .map(|(rel, _)| {
            let path = workdir.join(rel);
            Ok((rel.to_string(), std::fs::read_to_string(path)?))
        })
        .collect()
}

fn has_event_item_type(events: &[serde_json::Value], item_type: &str) -> bool {
    events.iter().any(|event| {
        event
            .get("item")
            .and_then(|item| item.get("type"))
            .and_then(|ty| ty.as_str())
            == Some(item_type)
    })
}

fn has_tool_use_evidence(results: &[e2e_codex::CodexRunResult]) -> bool {
    results.iter().any(|result| {
        has_event_item_type(&result.stdout_jsonl, "command_execution")
            || has_event_item_type(&result.stdout_jsonl, "file_change")
    })
}

fn has_compaction_evidence(home: &std::path::Path, results: &[e2e_codex::CodexRunResult]) -> bool {
    if results
        .iter()
        .any(|result| result.stderr.contains("context compacted"))
    {
        return true;
    }
    scan_tree_for_needles(
        home,
        &[
            "context compacted",
            "\"Compacted\"",
            "\"replacement_history\"",
        ],
    )
}

fn scan_tree_for_needles(root: &std::path::Path, needles: &[&str]) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let entry_path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(entry_path);
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.len() > 1_000_000 {
                continue;
            }
            let Ok(bytes) = std::fs::read(&entry_path) else {
                continue;
            };
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            if needles.iter().any(|needle| text.contains(needle)) {
                return true;
            }
        }
    }
    false
}

fn provider_supports_compaction_evidence(profile: &str) -> bool {
    match profile {
        "deepseek-chat" => {
            ProviderCapabilities::deepseek_chat().reliable_stream_usage_for_compaction
        }
        "fireworks-chat" => {
            ProviderCapabilities::fireworks_chat().reliable_stream_usage_for_compaction
        }
        "openrouter-chat" => {
            ProviderCapabilities::openrouter_chat().reliable_stream_usage_for_compaction
        }
        "openrouter-responses" => {
            ProviderCapabilities::openrouter_responses().reliable_stream_usage_for_compaction
        }
        "groq-chat" => ProviderCapabilities::groq_chat().reliable_stream_usage_for_compaction,
        "together-chat" => {
            ProviderCapabilities::together_chat().reliable_stream_usage_for_compaction
        }
        "generic-chat" => ProviderCapabilities::generic_chat().reliable_stream_usage_for_compaction,
        _ => true,
    }
}

async fn run_multiturn_script(
    codex_home: &std::path::Path,
    workdir: &std::path::Path,
    prompts: &[&str],
    sandbox: &str,
    json_output: bool,
) -> anyhow::Result<Vec<e2e_codex::CodexRunResult>> {
    let mut results = Vec::with_capacity(prompts.len());
    for (idx, prompt) in prompts.iter().enumerate() {
        let options = CodexExecOptions {
            sandbox: sandbox.to_string(),
            ephemeral: false,
            resume_last: idx > 0,
            json_output,
        };
        results
            .push(run_codex_exec_with_options(codex_home, workdir, prompt, &[], &options).await?);
    }
    Ok(results)
}

fn assert_scenario_outcome(
    label: &str,
    scenario: &DifferentialScenario,
    results: &[e2e_codex::CodexRunResult],
    workdir: &std::path::Path,
    codex_home: &std::path::Path,
) -> anyhow::Result<()> {
    if results.len() != scenario.expected_turn_messages.len() {
        anyhow::bail!(
            "[{label}/{}] expected {} turn results, got {}",
            scenario.name,
            scenario.expected_turn_messages.len(),
            results.len()
        );
    }

    for (idx, (result, expected)) in results
        .iter()
        .zip(scenario.expected_turn_messages.iter())
        .enumerate()
    {
        if !result.status.success() {
            anyhow::bail!(
                "[{label}/{}] turn {} failed with exit code {:?}\n--- stderr tail ---\n{}",
                scenario.name,
                idx + 1,
                result.status.code(),
                tail_lines(&result.stderr, 80),
            );
        }
        if scenario.require_exact_turn_messages && result.last_message.trim() != *expected {
            anyhow::bail!(
                "[{label}/{}] turn {} last_message mismatch\nexpected: {:?}\nactual: {:?}\n--- stderr tail ---\n{}",
                scenario.name,
                idx + 1,
                expected,
                result.last_message.trim(),
                tail_lines(&result.stderr, 80),
            );
        }
    }

    for (path, expected) in read_expected_files(workdir, scenario.expected_files)? {
        let wanted = scenario
            .expected_files
            .iter()
            .find(|(candidate, _)| *candidate == path)
            .map(|(_, content)| *content)
            .unwrap_or_default();
        if expected != wanted {
            anyhow::bail!(
                "[{label}/{}] file {} mismatch\nexpected: {:?}\nactual: {:?}",
                scenario.name,
                path,
                wanted,
                expected
            );
        }
    }

    if scenario.require_tool_evidence && !has_tool_use_evidence(results) {
        anyhow::bail!(
            "[{label}/{}] expected tool-use evidence in JSONL events",
            scenario.name
        );
    }

    if scenario.require_compaction_evidence && !has_compaction_evidence(codex_home, results) {
        anyhow::bail!(
            "[{label}/{}] expected compaction evidence in stderr or persisted session artifacts",
            scenario.name
        );
    }

    Ok(())
}

async fn run_differential_scenario(
    scenario: &DifferentialScenario,
    case: &ProviderCase,
    shim_base_url: &str,
    baseline_model: &str,
    baseline_auth: &OpenAiBaselineAuth,
) -> anyhow::Result<()> {
    if scenario.require_compaction_evidence && !provider_supports_compaction_evidence(&case.profile)
    {
        println!(
            "[{}:{}] skipping compaction scenario because the provider does not surface streaming usage totals",
            case.profile, scenario.name
        );
        return Ok(());
    }

    let tmp = create_workspace_tempdir(&format!("diff-{}-", case.profile))?;
    let baseline_root = tmp.path().join("baseline");
    let target_root = tmp.path().join("target");
    std::fs::create_dir_all(&baseline_root)?;
    std::fs::create_dir_all(&target_root)?;

    let fixture = create_fixture_dir(tmp.path(), scenario.initial_files)?;
    let baseline_workdir = baseline_root.join("workdir");
    let target_workdir = target_root.join("workdir");
    copy_dir_all(&fixture, &baseline_workdir)?;
    copy_dir_all(&fixture, &target_workdir)?;

    let baseline_catalog = match baseline_auth {
        OpenAiBaselineAuth::ApiKey => build_live_catalog_json(
            baseline_model,
            scenario.context_window,
            scenario.auto_compact_token_limit,
        ),
        OpenAiBaselineAuth::ChatGptAuth { .. } => build_live_catalog_json_with_base_instructions(
            baseline_model,
            scenario.context_window,
            scenario.auto_compact_token_limit,
            OPENAI_AUTH_BASE_INSTRUCTIONS,
        ),
    };
    let target_catalog = build_live_catalog_json(
        &case.model,
        scenario.context_window,
        scenario.auto_compact_token_limit,
    );

    let baseline_home = match baseline_auth {
        OpenAiBaselineAuth::ApiKey => generate_codex_home_with_provider(
            &baseline_root,
            "openai-direct",
            &openai_direct_provider_block(300000),
            baseline_model,
            &baseline_catalog,
        )?,
        OpenAiBaselineAuth::ChatGptAuth { auth_json_path } => {
            let home = generate_codex_home_default_provider(
                &baseline_root,
                baseline_model,
                &baseline_catalog,
            )?;
            seed_codex_home_auth(&home, auth_json_path)?;
            home
        }
    };
    let target_home = generate_codex_home_with_provider(
        &target_root,
        "local-shim",
        &local_shim_provider_block(shim_base_url, 300000),
        &case.model,
        &target_catalog,
    )?;

    let baseline_results = run_multiturn_script(
        &baseline_home,
        &baseline_workdir,
        scenario.prompts,
        scenario.sandbox,
        scenario.json_output,
    )
    .await?;
    let target_results = run_multiturn_script(
        &target_home,
        &target_workdir,
        scenario.prompts,
        scenario.sandbox,
        scenario.json_output,
    )
    .await?;

    assert_scenario_outcome(
        baseline_auth.label(),
        scenario,
        &baseline_results,
        &baseline_workdir,
        &baseline_home,
    )?;
    assert_scenario_outcome(
        &format!("target-{}", case.profile),
        scenario,
        &target_results,
        &target_workdir,
        &target_home,
    )?;

    if scenario.compare_final_messages {
        let baseline_final = baseline_results
            .last()
            .map(|result| result.last_message.trim())
            .unwrap_or_default();
        let target_final = target_results
            .last()
            .map(|result| result.last_message.trim())
            .unwrap_or_default();
        if baseline_final != target_final {
            anyhow::bail!(
                "[{}:{}] baseline/target final messages diverged\nbaseline: {:?}\ntarget: {:?}",
                case.profile,
                scenario.name,
                baseline_final,
                target_final
            );
        }
    }

    let baseline_files = read_expected_files(&baseline_workdir, scenario.expected_files)?;
    let target_files = read_expected_files(&target_workdir, scenario.expected_files)?;
    if baseline_files != target_files {
        anyhow::bail!(
            "[{}:{}] baseline/target file state diverged\nbaseline: {:?}\ntarget: {:?}",
            case.profile,
            scenario.name,
            baseline_files,
            target_files
        );
    }

    Ok(())
}

fn differential_scenarios() -> Vec<DifferentialScenario> {
    const EMPTY_FILES: &[(&str, &str)] = &[];
    vec![
        DifferentialScenario {
            name: "no_tool",
            initial_files: EMPTY_FILES,
            prompts: &[concat!(
                "Reply with exactly DIFF_NO_TOOL_OK and nothing else. ",
                "Do not inspect files. Do not run commands."
            )],
            expected_turn_messages: &["DIFF_NO_TOOL_OK"],
            require_exact_turn_messages: true,
            expected_files: EMPTY_FILES,
            sandbox: "read-only",
            require_tool_evidence: false,
            require_compaction_evidence: false,
            context_window: 131072,
            auto_compact_token_limit: None,
            json_output: true,
            compare_final_messages: true,
        },
        DifferentialScenario {
            name: "tool_read",
            initial_files: &[("answer.txt", "DIFF_TOOL_READ_OK\n")],
            prompts: &[concat!(
                "Read ./answer.txt using the shell, then reply with exactly its full contents. ",
                "Do not add any extra text."
            )],
            expected_turn_messages: &["DIFF_TOOL_READ_OK"],
            require_exact_turn_messages: true,
            expected_files: &[("answer.txt", "DIFF_TOOL_READ_OK\n")],
            sandbox: "read-only",
            require_tool_evidence: true,
            require_compaction_evidence: false,
            context_window: 131072,
            auto_compact_token_limit: None,
            json_output: true,
            compare_final_messages: true,
        },
        DifferentialScenario {
            name: "tool_write",
            initial_files: &[("target.txt", "before\n")],
            prompts: &[concat!(
                "Change ./target.txt so its entire contents are exactly ",
                "DIFF_TOOL_WRITE_OK followed by a trailing newline. ",
                "Then reply with exactly DONE."
            )],
            expected_turn_messages: &["DONE"],
            require_exact_turn_messages: false,
            expected_files: &[("target.txt", "DIFF_TOOL_WRITE_OK\n")],
            sandbox: "workspace-write",
            require_tool_evidence: true,
            require_compaction_evidence: false,
            context_window: 131072,
            auto_compact_token_limit: None,
            json_output: true,
            compare_final_messages: false,
        },
        DifferentialScenario {
            name: "multi_turn_no_tool",
            initial_files: EMPTY_FILES,
            prompts: &[
                "Remember the token DIFF_MULTI_TURN_OK. Reply with exactly ACK and nothing else. Do not include punctuation.",
                "What token did I ask you to remember? Reply with exactly DIFF_MULTI_TURN_OK.",
            ],
            expected_turn_messages: &["ACK", "DIFF_MULTI_TURN_OK"],
            require_exact_turn_messages: true,
            expected_files: EMPTY_FILES,
            sandbox: "read-only",
            require_tool_evidence: false,
            require_compaction_evidence: false,
            context_window: 131072,
            auto_compact_token_limit: None,
            json_output: true,
            compare_final_messages: true,
        },
        DifferentialScenario {
            name: "multi_turn_tool",
            initial_files: &[("note.txt", "DIFF_MULTI_TOOL_OK\n")],
            prompts: &[
                concat!(
                    "Read ./note.txt using the shell and remember its exact contents for the next turn. ",
                    "Reply with exactly STORED and nothing else."
                ),
                "What exact contents did you read earlier? Reply with exactly those contents and nothing else.",
            ],
            expected_turn_messages: &["STORED", "DIFF_MULTI_TOOL_OK"],
            expected_files: &[("note.txt", "DIFF_MULTI_TOOL_OK\n")],
            require_exact_turn_messages: true,
            sandbox: "read-only",
            require_tool_evidence: true,
            require_compaction_evidence: false,
            context_window: 131072,
            auto_compact_token_limit: None,
            json_output: true,
            compare_final_messages: true,
        },
        DifferentialScenario {
            name: "forced_compaction",
            initial_files: EMPTY_FILES,
            prompts: &[
                concat!(
                    "Remember the token DIFF_COMPACT_OK and reply with exactly ACK_COMPACT. ",
                    "Additional filler to force local compaction on the next turn: ",
                    "FILLER001 FILLER002 FILLER003 FILLER004 FILLER005 FILLER006 FILLER007 FILLER008 FILLER009 FILLER010 ",
                    "FILLER011 FILLER012 FILLER013 FILLER014 FILLER015 FILLER016 FILLER017 FILLER018 FILLER019 FILLER020 ",
                    "FILLER021 FILLER022 FILLER023 FILLER024 FILLER025 FILLER026 FILLER027 FILLER028 FILLER029 FILLER030 ",
                    "FILLER031 FILLER032 FILLER033 FILLER034 FILLER035 FILLER036 FILLER037 FILLER038 FILLER039 FILLER040."
                ),
                "What token did I ask you to remember before? Reply with exactly DIFF_COMPACT_OK.",
            ],
            expected_turn_messages: &["ACK_COMPACT", "DIFF_COMPACT_OK"],
            require_exact_turn_messages: true,
            expected_files: EMPTY_FILES,
            sandbox: "read-only",
            require_tool_evidence: false,
            require_compaction_evidence: true,
            context_window: 512,
            auto_compact_token_limit: Some(200),
            json_output: false,
            compare_final_messages: true,
        },
    ]
}

// ── Single-provider runner ───────────────────────────────────────

async fn run_one_live_provider(name: &str, case: &ProviderCase) -> anyhow::Result<()> {
    let tmp = tempfile::tempdir()?;
    let workdir = tempfile::tempdir()?;

    let config_path = tmp.path().join("shim_live.yaml");
    let chat_path = case.chat_path.as_deref().unwrap_or("/chat/completions");
    let responses_path = case.responses_path.as_deref().unwrap_or("/responses");

    let config_yaml = format!(
        r#"
server:
  listen: "127.0.0.1:__PORT__"
  base_path: "/v1"
  auth:
    mode: optional-bearer
    accepted_bearer_tokens:
      - "local-shim-test-token"

upstream:
  base_url: "{base_url}"
  chat_path: "{chat_path}"
  responses_path: "{responses_path}"
  models_path: "/models"
  api_key_env: "{api_key_env}"
  timeout_seconds: 180
  connect_timeout_seconds: 30
  max_retries: 1
{http_headers}
{query_params}

provider:
  kind: {profile}

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

logging:
  level: info
  redact_api_keys: true
  redact_message_content: true
"#,
        base_url = case.base_url,
        chat_path = chat_path,
        responses_path = responses_path,
        api_key_env = case.api_key_env,
        profile = case.profile,
        model = case.model,
        http_headers = if case.http_headers.is_empty() {
            String::new()
        } else {
            let mut s = String::from("  http_headers:\n");
            for (k, v) in &case.http_headers {
                s.push_str(&format!("    {k}: \"{v}\"\n"));
            }
            s
        },
        query_params = if case.query_params.is_empty() {
            String::new()
        } else {
            let mut s = String::from("  query_params:\n");
            for (k, v) in &case.query_params {
                s.push_str(&format!("    {k}: \"{v}\"\n"));
            }
            s
        },
    );

    std::fs::write(&config_path, config_yaml.trim())?;

    let shim =
        ShimProcess::start_with_env(&config_path, &[(&case.api_key_env, &case.api_key)]).await?;

    // Use bare catalog (no reasoning, no web_search) for smoke tests
    let codex_home = generate_codex_home_bare(tmp.path(), &shim.base_url(), &case.model)?;

    let prompt = "Return exactly the string CODEX_SHIM_E2E_OK and nothing else. Do not inspect files. Do not run commands.";

    let result = run_codex_exec(&codex_home, workdir.path(), prompt, &[]).await?;

    drop(shim);

    if !result.status.success() {
        anyhow::bail!(
            "[{name}] codex exit code {}\n\
             --- stderr (last 120 lines) ---\n{}\n\
             --- last_message ---\n{}\n\
             --- JSONL event types ---\n{:?}",
            result.status.code().unwrap_or(-1),
            tail_lines(&result.stderr, 120),
            result.last_message,
            result
                .stdout_jsonl
                .iter()
                .filter_map(|ev| ev.get("type").and_then(|v| v.as_str()))
                .collect::<Vec<_>>(),
        );
    }

    if !result.last_message.contains("CODEX_SHIM_E2E_OK") {
        anyhow::bail!(
            "[{name}] last_message does not contain CODEX_SHIM_E2E_OK: {}\n\
             --- stderr tail ---\n{}",
            result.last_message,
            tail_lines(&result.stderr, 40),
        );
    }

    println!(
        "[{name}] OK — model={} profile={}",
        case.model, case.profile
    );
    Ok(())
}

// ── Main matrix test ─────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires real provider keys and network"]
async fn live_provider_matrix_no_tool_smoke() {
    let path = std::env::var("CODEX_SHIM_E2E_KEYS")
        .expect("set CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml");

    let matrix =
        read_provider_matrix(&path).unwrap_or_else(|e| panic!("Failed to read {path}: {e}"));

    let enabled: Vec<_> = matrix.providers.iter().filter(|(_, c)| c.enabled).collect();

    assert!(!enabled.is_empty(), "no enabled providers in {path}");

    let mut failures = Vec::new();
    let mut successes = 0u32;
    let total = enabled.len();

    for (name, case) in &enabled {
        match run_one_live_provider(name, case).await {
            Ok(()) => successes += 1,
            Err(e) => {
                eprintln!("FAIL [{name}]: {e}");
                failures.push((name.to_string(), e.to_string()));
            }
        }
    }

    println!("\n=== Results: {successes}/{total} passed ===");
    for (name, err) in &failures {
        eprintln!("  [{name}]: {err}");
    }

    assert!(
        failures.is_empty(),
        "{}/{} providers failed: {:?}",
        failures.len(),
        total,
        failures.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
    );
}

// ── Optional tool smoke ──────────────────────────────────────────

#[tokio::test]
#[ignore = "requires real provider keys and network; set CODEX_SHIM_E2E_SKIP_TOOL_SMOKE=1 to skip intentionally"]
async fn live_provider_tool_smoke() {
    if should_skip_live_tool_tests() {
        eprintln!("Skipping live_provider_tool_smoke because CODEX_SHIM_E2E_SKIP_TOOL_SMOKE=1.");
        return;
    }

    let path = std::env::var("CODEX_SHIM_E2E_KEYS")
        .expect("set CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml");

    let matrix = read_provider_matrix(&path).unwrap();
    let enabled: Vec<_> = matrix.providers.iter().filter(|(_, c)| c.enabled).collect();
    assert!(!enabled.is_empty(), "no enabled providers in {path}");

    let mut failures = Vec::new();
    let mut successes = 0u32;
    let total = enabled.len();

    for (name, case) in enabled {
        let tmp = create_workspace_tempdir(&format!("tool-{name}-")).unwrap();
        let workdir = create_workspace_tempdir(&format!("tool-workdir-{name}-")).unwrap();

        std::fs::write(workdir.path().join("answer.txt"), "CODEX_SHIM_TOOL_OK").unwrap();

        let config_path = tmp.path().join("shim_live.yaml");
        let config_yaml = format!(
            r#"
server:
  listen: "127.0.0.1:__PORT__"
  base_path: "/v1"
  auth:
    mode: optional-bearer
    accepted_bearer_tokens: ["local-shim-test-token"]

upstream:
  base_url: "{base_url}"
  chat_path: "{chat_path}"
  responses_path: "{responses_path}"
  models_path: "/models"
  api_key_env: "{api_key_env}"
  timeout_seconds: 180
  connect_timeout_seconds: 30
  max_retries: 1

provider:
  kind: {profile}

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

state: {{ backend: memory }}

logging:
  level: info
  redact_api_keys: true
  redact_message_content: true
"#,
            base_url = case.base_url,
            chat_path = case.chat_path.as_deref().unwrap_or("/chat/completions"),
            responses_path = case.responses_path.as_deref().unwrap_or("/responses"),
            api_key_env = case.api_key_env,
            profile = case.profile,
            model = case.model,
        );

        std::fs::write(&config_path, config_yaml.trim()).unwrap();

        let outcome = async {
            let shim =
                ShimProcess::start_with_env(&config_path, &[(&case.api_key_env, &case.api_key)])
                    .await?;

            let codex_home = generate_codex_home_bare(tmp.path(), &shim.base_url(), &case.model)?;
            let prompt =
                "Read the file answer.txt using the shell, then reply with exactly its content.";
            let result = run_codex_exec(&codex_home, workdir.path(), prompt, &[]).await?;

            drop(shim);

            if !result.status.success() {
                anyhow::bail!(
                    "[{name}] tool smoke exit code should be 0 (got {:?})\n\
                     --- stderr tail ---\n{}\n\
                     --- last_message ---\n{}\n\
                     --- JSONL item types ---\n{:?}",
                    result.status.code(),
                    tail_lines(&result.stderr, 80),
                    result.last_message,
                    result
                        .stdout_jsonl
                        .iter()
                        .filter_map(|event| {
                            event
                                .get("item")
                                .and_then(|item| item.get("type"))
                                .and_then(|value| value.as_str())
                        })
                        .collect::<Vec<_>>(),
                );
            }
            if !result.last_message.contains("CODEX_SHIM_TOOL_OK") {
                anyhow::bail!(
                    "[{name}] last_message should contain CODEX_SHIM_TOOL_OK: {:?}\n\
                     --- stderr tail ---\n{}",
                    result.last_message,
                    tail_lines(&result.stderr, 80),
                );
            }
            if !has_tool_use_evidence(std::slice::from_ref(&result)) {
                anyhow::bail!(
                    "[{name}] JSONL should contain a tool execution event\n\
                     --- stderr tail ---\n{}\n\
                     --- JSONL ---\n{}",
                    tail_lines(&result.stderr, 80),
                    result
                        .stdout_jsonl
                        .iter()
                        .map(serde_json::Value::to_string)
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }

            println!("[{name}] tool smoke OK — model={}", case.model);
            Ok::<(), anyhow::Error>(())
        }
        .await;

        match outcome {
            Ok(()) => successes += 1,
            Err(err) => {
                eprintln!("FAIL [{name}] {err}");
                failures.push((name.to_string(), err.to_string()));
            }
        }
    }

    println!("\n=== Tool Results: {successes}/{total} passed ===");
    for (name, err) in &failures {
        eprintln!("  [{name}]: {err}");
    }

    assert!(
        failures.is_empty(),
        "{}/{} providers failed tool smoke: {:?}",
        failures.len(),
        total,
        failures.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()
    );
}

#[tokio::test]
#[ignore = "requires CODEX_SHIM_E2E_OPENAI_MODEL, provider keys, network, and codex sandbox helper; OPENAI_API_KEY is preferred but a Codex auth cache can be used as a fallback"]
async fn live_provider_differential_matrix() {
    if should_skip_live_tool_tests() {
        eprintln!(
            "Skipping live_provider_differential_matrix because CODEX_SHIM_E2E_SKIP_TOOL_SMOKE=1."
        );
        return;
    }

    let baseline_model = std::env::var("CODEX_SHIM_E2E_OPENAI_MODEL")
        .expect("set CODEX_SHIM_E2E_OPENAI_MODEL to an OpenAI Responses model slug");
    let baseline_auth = resolve_openai_baseline_auth()
        .unwrap_or_else(|err| panic!("failed to resolve OpenAI baseline auth: {err}"));

    let path = std::env::var("CODEX_SHIM_E2E_KEYS")
        .expect("set CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml");
    let matrix = read_provider_matrix(&path).unwrap();

    let enabled: Vec<_> = matrix.providers.iter().filter(|(_, c)| c.enabled).collect();
    assert!(!enabled.is_empty(), "no enabled providers in {path}");

    let scenarios = differential_scenarios();
    let mut failures = Vec::new();

    for (name, case) in enabled {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("shim_live_diff.yaml");
        let chat_path = case.chat_path.as_deref().unwrap_or("/chat/completions");
        let responses_path = case.responses_path.as_deref().unwrap_or("/responses");

        let config_yaml = format!(
            r#"
server:
  listen: "127.0.0.1:__PORT__"
  base_path: "/v1"
  auth:
    mode: optional-bearer
    accepted_bearer_tokens: ["local-shim-test-token"]

upstream:
  base_url: "{base_url}"
  chat_path: "{chat_path}"
  responses_path: "{responses_path}"
  models_path: "/models"
  api_key_env: "{api_key_env}"
  timeout_seconds: 180
  connect_timeout_seconds: 30
  max_retries: 1
{http_headers}
{query_params}

provider:
  kind: {profile}

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

logging:
  level: info
  redact_api_keys: true
  redact_message_content: true
"#,
            base_url = case.base_url,
            chat_path = chat_path,
            responses_path = responses_path,
            api_key_env = case.api_key_env,
            profile = case.profile,
            model = case.model,
            http_headers = if case.http_headers.is_empty() {
                String::new()
            } else {
                let mut s = String::from("  http_headers:\n");
                for (k, v) in &case.http_headers {
                    s.push_str(&format!("    {k}: \"{v}\"\n"));
                }
                s
            },
            query_params = if case.query_params.is_empty() {
                String::new()
            } else {
                let mut s = String::from("  query_params:\n");
                for (k, v) in &case.query_params {
                    s.push_str(&format!("    {k}: \"{v}\"\n"));
                }
                s
            },
        );
        std::fs::write(&config_path, config_yaml.trim()).unwrap();

        let shim = ShimProcess::start_with_env(&config_path, &[(&case.api_key_env, &case.api_key)])
            .await
            .unwrap_or_else(|err| panic!("failed to start shim for {name}: {err}"));

        for scenario in &scenarios {
            if let Err(err) = run_differential_scenario(
                scenario,
                case,
                &shim.base_url(),
                &baseline_model,
                &baseline_auth,
            )
            .await
            {
                failures.push((format!("{name}:{}", scenario.name), err.to_string()));
            }
        }

        drop(shim);
    }

    if !failures.is_empty() {
        for (label, err) in &failures {
            eprintln!("FAIL [{label}] {err}");
        }
    }

    assert!(
        failures.is_empty(),
        "{} differential scenarios failed: {:?}",
        failures.len(),
        failures
            .iter()
            .map(|(label, _)| label.as_str())
            .collect::<Vec<_>>()
    );
}
