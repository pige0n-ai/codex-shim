// ── Mock Upstream E2E Tests ──────────────────────────────────────
//
// Run:  cargo test -p e2e-codex --test codex_mock
// These tests are fully offline — they use a mock upstream server.
//
// Tests that require `codex` binary are behind #[ignore] by default.
// Run them with: cargo test -p e2e-codex codex_mock_ -- --ignored

use e2e_codex::mock_upstream::{CapturedRequest, MockUpstream, Scenario};
use e2e_codex::{
    ShimProcess, generate_codex_home, generate_codex_home_project_trust_only, generate_shim_config,
    generate_shim_config_native, run_codex_exec, write_project_codex_config,
};
use protocol::models::{CatalogModelSpec, build_model_catalog};
use serde_json::Value;

// ── Helpers ──────────────────────────────────────────────────────

/// Default setup for ChatShim tests: ChatStreamText scenario.
async fn setup(
    shim_provider: &str,
    model: &str,
) -> anyhow::Result<(
    MockUpstream,
    ShimProcess,
    tempfile::TempDir,
    tempfile::TempDir,
)> {
    let mock = MockUpstream::start(Scenario::ChatStreamText {
        deltas: vec!["CODEX_".into(), "SHIM_E2E_OK".into()],
        model: model.into(),
    })
    .await?;
    let tmp = tempfile::tempdir()?;
    let workdir = tempfile::tempdir()?;
    let config_path = generate_shim_config(tmp.path(), &mock.base_url(), shim_provider, model)?;
    let shim = ShimProcess::start(&config_path).await?;
    Ok((mock, shim, tmp, workdir))
}

fn response_id_from_sse(body: &str) -> String {
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if value["type"] == "response.created"
            && let Some(id) = value["response"]["id"].as_str()
        {
            return id.to_string();
        }
    }
    panic!("stream did not contain a response.created id:\n{body}");
}

fn last_chat_request(requests: &[CapturedRequest]) -> &Value {
    &requests
        .iter()
        .filter(|r| r.method == "POST" && r.path == "/v1/chat/completions")
        .last()
        .expect("expected upstream chat request")
        .body
}

fn system_message_texts(body: &Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .expect("chat request should include messages")
        .iter()
        .filter(|message| message["role"] == "system")
        .map(|message| chat_content_text(&message["content"]))
        .collect()
}

fn chat_content_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    if let Some(parts) = content.as_array() {
        return parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n");
    }
    content.to_string()
}

fn assistant_tool_call_index(messages: &[Value], call_id: &str) -> usize {
    messages
        .iter()
        .position(|message| {
            message["role"] == "assistant"
                && message["tool_calls"]
                    .as_array()
                    .map(|tool_calls| tool_calls.iter().any(|tc| tc["id"] == call_id))
                    .unwrap_or(false)
        })
        .unwrap_or_else(|| panic!("missing assistant tool call {call_id}: {messages:#?}"))
}

fn assert_tool_result_immediately_after(messages: &[Value], assistant_idx: usize, call_id: &str) {
    let tool = messages
        .get(assistant_idx + 1)
        .unwrap_or_else(|| panic!("missing tool result after assistant: {messages:#?}"));
    assert_eq!(tool["role"], "tool");
    assert_eq!(tool["tool_call_id"], call_id);
}

fn exec_command_tool() -> Value {
    serde_json::json!({
        "type": "function",
        "name": "exec_command",
        "description": "Run a shell command",
        "parameters": {
            "type": "object",
            "properties": {
                "cmd": {"type": "string"}
            },
            "required": ["cmd"]
        }
    })
}

fn codex_home_with_base_instructions(
    base: &std::path::Path,
    shim_url: &str,
    model: &str,
    base_instructions: &str,
) -> anyhow::Result<std::path::PathBuf> {
    let caps = protocol::provider_caps::ProviderCapabilities {
        supports_function_tools: true,
        supports_parallel_tool_calls: true,
        supports_reasoning_effort: false,
        ..Default::default()
    };
    let catalog = serde_json::to_value(build_model_catalog(
        &[CatalogModelSpec {
            slug: model.to_string(),
            display_name: Some(model.to_string()),
            description: None,
            context_window: 131072,
            tool_calling: Some(true),
            vision: Some(false),
            reasoning_levels: Some(vec![]),
            priority: Some(10),
            base_instructions: Some(base_instructions.to_string()),
            auto_compact_token_limit: None,
            supports_search_tool: Some(false),
            supports_reasoning_summaries: Some(false),
            apply_patch_tool_type: None,
            supports_image_detail_original: Some(false),
        }],
        &caps,
    ))?;
    e2e_codex::generate_codex_home_with_provider(
        base,
        "local-shim",
        &format!(
            r#"[model_providers.local-shim]
name = "codex-shim"
base_url = "{shim_url}"
env_key = "LOCAL_SHIM_TOKEN"
wire_api = "responses"
supports_websockets = false
request_max_retries = 0
stream_max_retries = 0
stream_idle_timeout_ms = 120000
"#
        ),
        model,
        &catalog,
    )
}

// ── A. Codex subprocess smoke ────────────────────────────────────

#[tokio::test]
#[ignore = "requires codex binary"]
async fn codex_mock_chat_stream_basic() {
    let (mock, shim, tmp, workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let codex_home = generate_codex_home(tmp.path(), &shim.base_url(), "mock-model").unwrap();

    let result = run_codex_exec(
        &codex_home,
        workdir.path(),
        "Return exactly CODEX_SHIM_E2E_OK. Do not edit files.",
        &[],
    )
    .await
    .expect("codex exec");

    drop(shim);
    drop(mock);

    assert!(
        result.status.success(),
        "codex exit code should be 0, got {:?}\n--- stderr ---\n{}\n--- end stderr ---",
        result.status.code(),
        result.stderr,
    );
    assert!(
        result.last_message.contains("CODEX_SHIM_E2E_OK"),
        "last_message should contain CODEX_SHIM_E2E_OK, got: {}",
        result.last_message
    );

    // Verify JSONL contains events (exact event schema varies by Codex version)
    if !result.stdout_jsonl.is_empty() {
        let event_types: Vec<&str> = result
            .stdout_jsonl
            .iter()
            .filter_map(|ev| ev.get("type").and_then(|v| v.as_str()))
            .collect();
        eprintln!(
            "codex_mock_chat_stream_basic: {} JSONL events, types: {:?}",
            result.stdout_jsonl.len(),
            event_types
        );
    }
}

#[tokio::test]
#[ignore = "requires codex binary"]
async fn codex_mock_model_base_instructions_reach_upstream_system_prompt() {
    let sentinel = "BASE_INSTRUCTIONS_SENTINEL_9b0a8f";
    let (mock, shim, tmp, workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let codex_home =
        codex_home_with_base_instructions(tmp.path(), &shim.base_url(), "mock-model", sentinel)
            .expect("codex home with base instructions");

    let result = run_codex_exec(&codex_home, workdir.path(), "Reply with OK.", &[])
        .await
        .expect("codex exec");

    drop(shim);
    let requests = mock.state.take_requests();
    let chat_body = last_chat_request(&requests);
    let system_texts = system_message_texts(chat_body);

    drop(mock);

    assert!(
        result.status.success(),
        "codex exit code should be 0, got {:?}\n--- stderr ---\n{}\n--- end stderr ---",
        result.status.code(),
        result.stderr,
    );
    assert!(
        system_texts.iter().any(|text| text.contains(sentinel)),
        "expected base_instructions sentinel in upstream system messages: {system_texts:#?}\n\
         upstream body: {chat_body:#?}",
    );
}

// ── B. Request headers / query / auth assertions ─────────────────

#[tokio::test]
#[ignore = "requires codex binary"]
async fn codex_mock_request_builder_headers_query_auth() {
    let (mock, shim, tmp, workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let codex_home = generate_codex_home(tmp.path(), &shim.base_url(), "mock-model").unwrap();

    let _result = run_codex_exec(&codex_home, workdir.path(), "Say hello.", &[])
        .await
        .expect("codex exec");

    drop(shim);
    let requests = mock.state.take_requests();

    let chat_req = requests
        .iter()
        .find(|r| r.method == "POST" && r.path == "/v1/chat/completions");
    assert!(
        chat_req.is_some(),
        "mock upstream did not receive a chat completions request.\n\
         Codex stderr:\n{}\n--- end stderr ---\n\
         Captured {} upstream requests: {:?}",
        _result.stderr,
        requests.len(),
        requests
            .iter()
            .map(|r| format!("{} {}", r.method, r.path))
            .collect::<Vec<_>>(),
    );
    let chat_req = chat_req.unwrap();

    let auth = chat_req.headers.get("authorization");
    assert!(auth.is_some(), "should have Authorization header");
    assert!(
        auth.unwrap().starts_with("Bearer "),
        "should be Bearer token"
    );
    assert_eq!(
        chat_req.headers.get("x-e2e-static").map(|s| s.as_str()),
        Some("static-value")
    );
    assert_eq!(
        chat_req.headers.get("x-e2e-from-env").map(|s| s.as_str()),
        Some("dynamic-value")
    );
    assert_eq!(
        chat_req.query_params.get("api-version"),
        Some(&"e2e-test".to_string())
    );
    drop(mock);
}

#[tokio::test]
#[ignore = "requires codex binary"]
async fn codex_mock_project_config_trusted_basic() {
    let (mock, shim, tmp, workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let codex_home = generate_codex_home_project_trust_only(tmp.path(), workdir.path()).unwrap();
    write_project_codex_config(workdir.path(), &shim.base_url(), "mock-model")
        .expect("project config");

    let result = run_codex_exec(
        &codex_home,
        workdir.path(),
        "Return exactly CODEX_SHIM_E2E_OK. Do not edit files.",
        &[],
    )
    .await
    .expect("codex exec");

    drop(shim);
    drop(mock);

    assert!(
        result.status.success(),
        "codex exit code should be 0, got {:?}\n--- stderr ---\n{}\n--- end stderr ---",
        result.status.code(),
        result.stderr,
    );
    assert!(
        result.last_message.contains("CODEX_SHIM_E2E_OK"),
        "last_message should contain CODEX_SHIM_E2E_OK, got: {}",
        result.last_message
    );
}

// ── B2. Direct HTTP request builder (non-ignored) ────────────────

#[tokio::test]
async fn direct_request_builder_headers_query_auth() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
            "stream": true
        }))
        .send()
        .await
        .expect("request");

    assert!(resp.status().is_success());
    let _body = resp.text().await.unwrap();

    drop(shim);
    let requests = mock.state.take_requests();

    let chat_req = requests
        .iter()
        .find(|r| r.method == "POST" && r.path == "/v1/chat/completions");
    assert!(
        chat_req.is_some(),
        "mock upstream did not receive a chat completions request via direct HTTP.\n\
         Captured {} upstream requests: {:?}",
        requests.len(),
        requests
            .iter()
            .map(|r| format!("{} {}", r.method, r.path))
            .collect::<Vec<_>>(),
    );
    let chat_req = chat_req.unwrap();

    // Auth
    let auth = chat_req.headers.get("authorization");
    assert!(
        auth.is_some(),
        "upstream request should have Authorization header"
    );
    assert!(
        auth.unwrap().contains("mock-api-key-for-testing"),
        "Authorization should contain the mock API key"
    );

    // Static header
    assert_eq!(
        chat_req.headers.get("x-e2e-static").map(|s| s.as_str()),
        Some("static-value"),
        "static header X-E2E-Static should be forwarded"
    );

    // Env header
    assert_eq!(
        chat_req.headers.get("x-e2e-from-env").map(|s| s.as_str()),
        Some("dynamic-value"),
        "env header X-E2E-From-Env should be forwarded"
    );

    // Query param
    assert!(
        chat_req.query_params.contains_key("api-version"),
        "query params should contain api-version"
    );
    assert_eq!(
        chat_req.query_params.get("api-version"),
        Some(&"e2e-test".to_string())
    );

    drop(mock);
}

#[tokio::test]
async fn direct_instructions_reach_upstream_system_prompt() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();
    let sentinel = "DIRECT_INSTRUCTIONS_SENTINEL_4c6502";

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "instructions": sentinel,
            "input": "hi",
            "stream": true
        }))
        .send()
        .await
        .expect("request");

    assert!(resp.status().is_success());
    let _body = resp.text().await.unwrap();

    drop(shim);
    let requests = mock.state.take_requests();
    let chat_body = last_chat_request(&requests);
    let system_texts = system_message_texts(chat_body);

    assert!(
        system_texts.iter().any(|text| text.contains(sentinel)),
        "expected instructions sentinel in upstream system messages: {system_texts:#?}\n\
         upstream body: {chat_body:#?}",
    );
    drop(mock);
}

// ── C. Unsupported fields (direct HTTP) ──────────────────────────

#[tokio::test]
async fn direct_unsupported_fields_fail_closed() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
            "background": true
        }))
        .send()
        .await
        .expect("request");

    drop(shim);
    drop(mock);

    assert_eq!(resp.status().as_u16(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "not_implemented");
    assert_eq!(body["error"]["param"], "background");
}

#[tokio::test]
async fn direct_truncation_field_rejected() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
            "truncation": "auto"
        }))
        .send()
        .await
        .expect("request");

    drop(shim);
    drop(mock);

    assert_eq!(resp.status().as_u16(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "not_implemented");
    assert_eq!(body["error"]["param"], "truncation");
}

#[tokio::test]
async fn direct_conversation_field_rejected() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
            "conversation": "conv_123"
        }))
        .send()
        .await
        .expect("request");

    drop(shim);
    drop(mock);

    assert_eq!(resp.status().as_u16(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "not_implemented");
    assert_eq!(body["error"]["param"], "conversation");
}

// ── D. Native stream store id ────────────────────────────────────

#[tokio::test]
async fn direct_native_stream_store_id() {
    let resp_id = "resp_upstream_123";
    let mock = MockUpstream::start(Scenario::ResponsesStreamCompleted {
        response_id: resp_id.into(),
        output_text: "NATIVE_OK".into(),
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let config_path = generate_shim_config_native(
        tmp.path(),
        &mock.base_url(),
        "openrouter-responses",
        "mock-model",
    )
    .unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
            "stream": true
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let stream_body = resp.text().await.unwrap();
    assert!(
        stream_body.contains("response.completed"),
        "stream should contain response.completed"
    );

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let get_resp = client
        .get(format!("{}/responses/{resp_id}", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .send()
        .await
        .unwrap();

    assert!(
        get_resp.status().is_success(),
        "GET /v1/responses/{resp_id} should succeed"
    );
    let stored: serde_json::Value = get_resp.json().await.unwrap();
    assert_eq!(stored["id"], resp_id);
    assert_eq!(stored["status"], "completed");

    drop(shim);
    drop(mock);
}

// ── E. Native stream abnormal end ────────────────────────────────

#[tokio::test]
async fn direct_native_stream_abnormal_end() {
    let resp_id = "resp_abnormal_1";
    let mock = MockUpstream::start(Scenario::ResponsesStreamAbnormalEnd {
        response_id: resp_id.into(),
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let config_path = generate_shim_config_native(
        tmp.path(),
        &mock.base_url(),
        "openrouter-responses",
        "mock-model",
    )
    .unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let stream_body = resp.text().await.unwrap();
    assert!(
        stream_body.contains("response.created"),
        "stream should have at least response.created"
    );
    assert!(
        !stream_body.contains("response.completed"),
        "abnormal stream should NOT have response.completed"
    );

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let get_resp = client
        .get(format!("{}/responses/{resp_id}", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .send()
        .await
        .unwrap();

    assert_eq!(
        get_resp.status().as_u16(),
        404,
        "abnormal-end response should not be stored"
    );

    drop(shim);
    drop(mock);
}

// ── F. Stateless continuation (previous_response_id) ─────────────

#[tokio::test]
async fn direct_stateless_previous_response_id_materialization() {
    let first_id = "resp_first_1";
    let mock = MockUpstream::start(Scenario::ResponsesNonStream {
        response_id: first_id.into(),
        output_text: "Hello from first response".into(),
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();

    let config_path = generate_shim_config_native(
        tmp.path(),
        &mock.base_url(),
        "openrouter-responses",
        "mock-model",
    )
    .unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();
    let auth = "local-shim-test-token";

    let resp1 = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "What is the answer?",
        }))
        .send()
        .await
        .unwrap();
    assert!(resp1.status().is_success());

    mock.state.set_scenario(Scenario::ResponsesNonStream {
        response_id: "resp_second_2".into(),
        output_text: "Follow-up answer".into(),
        model: "mock-model".into(),
    });

    let resp2 = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "Tell me more",
            "previous_response_id": first_id,
        }))
        .send()
        .await
        .unwrap();
    assert!(resp2.status().is_success());

    let requests = mock.state.take_requests();
    let second_req = requests
        .iter()
        .filter(|r| r.path == "/v1/responses" && r.method == "POST")
        .last()
        .expect("second responses request");

    let input = second_req.body["input"].as_array().unwrap();
    assert!(
        input.len() >= 3,
        "input should have ≥3 items (history + new), got {}",
        input.len()
    );

    let input_json = serde_json::to_string(&second_req.body["input"]).unwrap();
    assert!(
        input_json.contains("Hello from first response"),
        "materialized history must contain the first assistant response text"
    );
    assert!(
        input_json.contains("Tell me more"),
        "materialized input must contain the new user message"
    );

    let has_assistant = input.iter().any(|item| {
        item.get("role")
            .and_then(|v| v.as_str())
            .map_or(false, |r| r == "assistant")
    });
    assert!(
        has_assistant,
        "materialized history should include an assistant message"
    );

    drop(shim);
    drop(mock);
}

// ── F2. Provider behavior quality gates ─────────────────────────

#[tokio::test]
async fn direct_deepseek_reasoning_recovery_preserves_tool_reasoning() {
    let mock = MockUpstream::start(Scenario::ChatStreamReasoningToolCall {
        reasoning: "need shell before calling exec_command".into(),
        tool_name: "exec_command".into(),
        tool_args: serde_json::json!({"cmd": "pwd"}),
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let config_path =
        generate_shim_config(tmp.path(), &mock.base_url(), "deepseek-chat", "mock-model").unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();
    let auth = "local-shim-test-token";

    let first = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "Use the exec_command tool.",
            "stream": true,
            "tools": [exec_command_tool()]
        }))
        .send()
        .await
        .unwrap();
    assert!(first.status().is_success());
    let first_body = first.text().await.unwrap();
    let first_response_id = response_id_from_sse(&first_body);
    assert!(
        first_body.contains("response.completed"),
        "first stream must complete before previous_response_id continuation"
    );

    mock.state.set_scenario(Scenario::ChatNonStreamText {
        text: "done".into(),
        model: "mock-model".into(),
    });

    let second = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "model": "mock-model",
            "previous_response_id": first_response_id,
            "input": [{
                "type": "function_call_output",
                "call_id": "call_mock_1",
                "output": "{\"stdout\":\"/tmp/project\"}"
            }],
            "stream": false,
            "tools": [exec_command_tool()]
        }))
        .send()
        .await
        .unwrap();
    assert!(second.status().is_success());
    let _second_body: Value = second.json().await.unwrap();

    drop(shim);
    let requests = mock.state.take_requests();
    let chat_req = last_chat_request(&requests);
    let messages = chat_req["messages"]
        .as_array()
        .expect("chat messages array");
    let assistant_idx = assistant_tool_call_index(messages, "call_mock_1");
    let assistant = &messages[assistant_idx];

    assert_eq!(
        assistant["reasoning_content"], "need shell before calling exec_command",
        "DeepSeek reasoning_content must survive previous_response_id tool continuation"
    );
    assert_tool_result_immediately_after(messages, assistant_idx, "call_mock_1");
    drop(mock);
}

#[tokio::test]
async fn direct_tool_call_adjacency_reorders_intervening_messages() {
    let mock = MockUpstream::start(Scenario::ChatNonStreamText {
        text: "ok".into(),
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let config_path =
        generate_shim_config(tmp.path(), &mock.base_url(), "deepseek-chat", "mock-model").unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": [
                {"type": "message", "role": "user", "content": "before"},
                {"type": "function_call", "call_id": "call_a", "name": "exec_command", "arguments": "{\"cmd\":\"pwd\"}"},
                {"type": "message", "role": "user", "content": "intervening status update"},
                {"type": "function_call_output", "call_id": "call_a", "output": "ok"}
            ],
            "stream": false,
            "tools": [exec_command_tool()]
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let _body: Value = resp.json().await.unwrap();

    drop(shim);
    let requests = mock.state.take_requests();
    let chat_req = last_chat_request(&requests);
    let messages = chat_req["messages"]
        .as_array()
        .expect("chat messages array");
    let assistant_idx = assistant_tool_call_index(messages, "call_a");

    assert_tool_result_immediately_after(messages, assistant_idx, "call_a");
    assert!(
        messages[..assistant_idx]
            .iter()
            .any(|message| message["role"] == "user"
                && message["content"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("intervening")),
        "intervening non-tool messages must move before the assistant tool call"
    );
    drop(mock);
}

#[tokio::test]
async fn direct_parallel_tool_calls_grouped_and_outputs_ordered() {
    let mock = MockUpstream::start(Scenario::ChatNonStreamText {
        text: "ok".into(),
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let config_path =
        generate_shim_config(tmp.path(), &mock.base_url(), "deepseek-chat", "mock-model").unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": [
                {"type": "message", "role": "user", "content": "run both commands"},
                {"type": "function_call", "call_id": "call_a", "name": "exec_command", "arguments": "{\"cmd\":\"pwd\"}"},
                {"type": "function_call", "call_id": "call_b", "name": "exec_command", "arguments": "{\"cmd\":\"ls\"}"},
                {"type": "function_call_output", "call_id": "call_b", "output": "ls output"},
                {"type": "function_call_output", "call_id": "call_a", "output": "pwd output"}
            ],
            "stream": false,
            "parallel_tool_calls": true,
            "tools": [exec_command_tool()]
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let _body: Value = resp.json().await.unwrap();

    drop(shim);
    let requests = mock.state.take_requests();
    let chat_req = last_chat_request(&requests);
    let messages = chat_req["messages"]
        .as_array()
        .expect("chat messages array");
    let assistant_idx = assistant_tool_call_index(messages, "call_a");
    let assistant = &messages[assistant_idx];
    let tool_calls = assistant["tool_calls"]
        .as_array()
        .expect("assistant tool_calls");

    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0]["id"], "call_a");
    assert_eq!(tool_calls[1]["id"], "call_b");
    assert_eq!(messages[assistant_idx + 1]["role"], "tool");
    assert_eq!(messages[assistant_idx + 1]["tool_call_id"], "call_a");
    assert_eq!(messages[assistant_idx + 2]["role"], "tool");
    assert_eq!(messages[assistant_idx + 2]["tool_call_id"], "call_b");
    drop(mock);
}

// ── G. Chat non-stream basic ─────────────────────────────────────

#[tokio::test]
async fn direct_chat_nonstream_basic() {
    let mock = MockUpstream::start(Scenario::ChatNonStreamText {
        text: "Hello from non-stream".into(),
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let config_path =
        generate_shim_config(tmp.path(), &mock.base_url(), "deepseek-chat", "mock-model").unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "Hello world",
            "stream": false
        }))
        .send()
        .await
        .unwrap();

    drop(shim);

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "completed");
    assert!(
        body["output_text"]
            .as_str()
            .unwrap()
            .contains("Hello from non-stream")
    );

    let requests = mock.state.take_requests();
    let chat_req = requests
        .iter()
        .find(|r| r.path == "/v1/chat/completions")
        .expect("should have called upstream chat");

    assert_eq!(chat_req.method, "POST");
    assert!(
        chat_req.body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Hello world")
    );

    drop(mock);
}

// ── H. Chat stream basic (direct HTTP) ───────────────────────────

#[tokio::test]
async fn direct_chat_stream_basic() {
    let mock = MockUpstream::start(Scenario::ChatStreamText {
        deltas: vec!["CODEX_".into(), "SHIM_E2E_OK".into()],
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let config_path =
        generate_shim_config(tmp.path(), &mock.base_url(), "deepseek-chat", "mock-model").unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "Hello",
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert!(resp.status().is_success());
    let body = resp.text().await.unwrap();
    assert!(body.contains("CODEX_"), "stream should contain first delta");
    assert!(
        body.contains("SHIM_E2E_OK"),
        "stream should contain second delta"
    );

    assert!(
        body.contains("response.completed"),
        "stream should contain response.completed"
    );
    assert!(
        body.contains("total_tokens"),
        "response.completed should contain usage with total_tokens"
    );

    drop(shim);
    drop(mock);
}

// ── I. Upstream 401 handling ─────────────────────────────────────

#[tokio::test]
async fn direct_upstream_401() {
    let mock = MockUpstream::start(Scenario::Upstream401).await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let config_path =
        generate_shim_config(tmp.path(), &mock.base_url(), "deepseek-chat", "mock-model").unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status().as_u16(),
        401,
        "should propagate 401 from upstream"
    );

    drop(shim);
    drop(mock);
}

// ── K. Fail-closed request validation ────────────────────────────

#[tokio::test]
async fn direct_include_field_is_rejected_fail_closed() {
    let mock = MockUpstream::start(Scenario::ChatNonStreamText {
        text: "should not be called".into(),
        model: "mock-model".into(),
    })
    .await
    .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let config_path =
        generate_shim_config(tmp.path(), &mock.base_url(), "deepseek-chat", "mock-model").unwrap();
    let shim = ShimProcess::start(&config_path).await.unwrap();
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
            "include": ["some.value"]
        }))
        .send()
        .await
        .expect("request");

    let body: serde_json::Value = resp.json().await.unwrap();

    drop(shim);

    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "include");
    assert_eq!(body["error"]["code"], "unsupported_include");
    assert!(
        mock.state.take_requests().is_empty(),
        "invalid include should fail before contacting upstream"
    );
    drop(mock);
}

#[tokio::test]
async fn direct_unknown_top_level_field_rejected() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": "hi",
            "future_parameter": true
        }))
        .send()
        .await
        .expect("request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();

    drop(shim);

    assert_eq!(status.as_u16(), 400);
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "future_parameter");
    assert_eq!(body["error"]["code"], "unknown_parameter");
    assert!(
        mock.state.take_requests().is_empty(),
        "unknown top-level fields should fail before contacting upstream"
    );
    drop(mock);
}

#[tokio::test]
async fn direct_unknown_input_item_type_rejected() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": {
                "type": "future_item",
                "payload": "hi"
            }
        }))
        .send()
        .await
        .expect("request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();

    drop(shim);

    assert_eq!(status.as_u16(), 400);
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "input");
    assert_eq!(body["error"]["code"], "unknown_input_item");
    assert!(
        mock.state.take_requests().is_empty(),
        "unknown input item types should fail before contacting upstream"
    );
    drop(mock);
}

#[tokio::test]
async fn direct_invalid_raw_input_object_rejected() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": {
                "role": "user",
                "content": "hi"
            }
        }))
        .send()
        .await
        .expect("request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();

    drop(shim);

    assert_eq!(status.as_u16(), 400);
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["param"], "input");
    assert_eq!(body["error"]["code"], "missing_type");
    assert!(
        mock.state.take_requests().is_empty(),
        "invalid raw input objects should fail before contacting upstream"
    );
    drop(mock);
}

#[tokio::test]
async fn direct_models_returns_shim_native_catalog() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/models", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .send()
        .await
        .expect("request");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();

    drop(shim);

    assert_eq!(body["models"][0]["slug"], "mock-model");
    assert_eq!(body["models"][0]["display_name"], "mock-model");
    assert_eq!(body["models"][0]["context_window"], 131072);
    assert!(body["models"][0]["supported_reasoning_levels"].is_array());
    assert_eq!(body["models"][0]["supports_parallel_tool_calls"], true);
    assert!(
        !mock
            .state
            .take_requests()
            .iter()
            .any(|r| r.path == "/v1/models"),
        "/models should be served by the shim, not proxied upstream"
    );
    drop(mock);
}

#[tokio::test]
async fn direct_compact_endpoint_not_implemented() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/responses/compact", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "model": "mock-model",
            "input": []
        }))
        .send()
        .await
        .expect("request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();

    drop(shim);

    assert_eq!(status.as_u16(), 400);
    assert_eq!(body["error"]["type"], "not_implemented");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("local compaction")
    );
    assert!(
        mock.state.take_requests().is_empty(),
        "unsupported compact endpoint should fail before contacting upstream"
    );
    drop(mock);
}

#[tokio::test]
async fn direct_memory_summarize_endpoint_not_implemented() {
    let (mock, shim, _tmp, _workdir) = setup("deepseek-chat", "mock-model").await.expect("setup");
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/memories/trace_summarize", shim.base_url()))
        .bearer_auth("local-shim-test-token")
        .json(&serde_json::json!({
            "messages": []
        }))
        .send()
        .await
        .expect("request");

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();

    drop(shim);

    assert_eq!(status.as_u16(), 400);
    assert_eq!(body["error"]["type"], "not_implemented");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("local state")
    );
    assert!(
        mock.state.take_requests().is_empty(),
        "unsupported memory summarization should fail before contacting upstream"
    );
    drop(mock);
}
