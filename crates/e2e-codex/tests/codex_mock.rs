// ── Mock Upstream E2E Tests ──────────────────────────────────────
//
// Run:  cargo test -p e2e-codex --test codex_mock
// These tests are fully offline — they use a mock upstream server.
//
// Tests that require `codex` binary are behind #[ignore] by default.
// Run them with: cargo test -p e2e-codex codex_mock_ -- --ignored

use e2e_codex::mock_upstream::{MockUpstream, Scenario};
use e2e_codex::{
    ShimProcess, generate_codex_home, generate_codex_home_project_trust_only, generate_shim_config,
    generate_shim_config_native, run_codex_exec, write_project_codex_config,
};

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
