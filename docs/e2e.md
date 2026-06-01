# E2E Testing for codex-shim

This document describes the end-to-end test suite for codex-shim,
which validates the full chain: **Codex → codex-shim → upstream**.

## Architecture

The test suite is split into two tiers:

| Tier | Crate/Target | Upstream | Requires |
|------|-------------|----------|----------|
| **Mock E2E** | `crates/e2e-codex/tests/codex_mock.rs` | Mock upstream (axum) | Nothing (offline) |
| **Live Smoke** | `crates/e2e-codex/tests/codex_live.rs` | Real provider APIs | API keys + network |

```
Codex exec (subprocess) → codex-shim (subprocess) → mock upstream / real provider
```

## Quick Start

### Mock E2E (offline, CI-safe)

`codex_mock` auto-builds `codex-shim` when needed. In CI, pre-building once and
setting `CODEX_SHIM_BIN` is faster and avoids repeated rebuilds.

```bash
# Run all mock tests (no codex binary needed)
cargo test -p e2e-codex --test codex_mock -- --nocapture

# Run tests that also exercise codex binary
cargo test -p e2e-codex --test codex_mock -- --ignored --nocapture
```

### Live Provider Smoke (requires keys)

```bash
# 1. Copy and fill in the secrets file
cp crates/e2e-codex/fixtures/keys.example.toml secrets/e2e.providers.toml
# Edit secrets/e2e.providers.toml: set enabled = true + add real api_key
chmod 600 secrets/e2e.providers.toml  # optional but recommended on macOS/Linux

# 2. Run
CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live -- --ignored --nocapture
```

### Live Differential Matrix (OpenAI baseline)

The differential suite needs an OpenAI baseline plus the provider matrix keys.

Preferred path:

```bash
OPENAI_API_KEY=sk-... \
CODEX_SHIM_E2E_OPENAI_MODEL=gpt-5.4-mini \
CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live live_provider_differential_matrix -- --ignored --nocapture
```

Fallback when you do not use an API key:

```bash
CODEX_SHIM_E2E_OPENAI_AUTH_JSON="${CODEX_HOME:-$HOME/.codex}/auth.json" \
CODEX_SHIM_E2E_OPENAI_MODEL=gpt-5.4-mini \
CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live live_provider_differential_matrix -- --ignored --nocapture
```

This fallback mirrors the official Codex CLI guidance for isolated environments:
copy a valid `auth.json` into the target `CODEX_HOME`. API keys remain the
preferred default for automation.

On Windows PowerShell, set the fallback path explicitly, for example:

```powershell
$env:CODEX_SHIM_E2E_OPENAI_AUTH_JSON = "$env:USERPROFILE\.codex\auth.json"
$env:CODEX_SHIM_E2E_OPENAI_MODEL = "gpt-5.4-mini"
$env:CODEX_SHIM_E2E_KEYS = "secrets/e2e.providers.toml"
cargo test -p e2e-codex --test codex_live live_provider_differential_matrix -- --ignored --nocapture
```

## Test Cases

### Mock E2E (`codex_mock.rs`)

| Test | Description | Requires codex |
|------|-------------|---------------|
| `direct_chat_nonstream_basic` | Chat non-stream → Responses output | No |
| `direct_chat_stream_basic` | Chat SSE stream → Responses SSE | No |
| `direct_unsupported_fields_fail_closed` | `background: true` → 400 not_implemented | No |
| `direct_truncation_field_rejected` | `truncation` → 400 not_implemented | No |
| `direct_conversation_field_rejected` | Unsupported `conversation` field fails closed | No |
| `direct_native_stream_store_id` | Native Responses stream stores upstream response id | No |
| `direct_native_stream_abnormal_end` | Abnormal stream doesn't write store record | No |
| `direct_stateless_previous_response_id_materialization` | previous_response_id materializes history | No |
| `direct_deepseek_reasoning_recovery_preserves_tool_reasoning` | Stored DeepSeek reasoning survives tool-call turns | No |
| `direct_tool_call_adjacency_reorders_intervening_messages` | Chat history keeps assistant/tool adjacency valid | No |
| `direct_parallel_tool_calls_grouped_and_outputs_ordered` | Parallel tool calls and outputs are grouped for chat history | No |
| `direct_chat_stream_retries_initial_429` | Streaming chat retries initial retryable upstream failures | No |
| `direct_chat_stream_reasoning_only_chunks_emit_downstream_heartbeat` | Heartbeat protects reasoning-only streams from idle timeout | No |
| `direct_chat_stream_downstream_heartbeat_can_be_disabled` | `downstream_heartbeat_seconds: 0` disables heartbeat emission | No |
| `direct_chat_stream_usage_only_chunk_can_emit_heartbeat_and_preserves_usage` | Usage-only chunks can heartbeat without losing usage accounting | No |
| `direct_chat_stream_custom_tool_argument_chunks_emit_heartbeat_without_partial_input` | Custom/freeform tool argument accumulation can heartbeat safely | No |
| `direct_chat_stream_mapper_failure_persists_debug_artifact` | Mapper failures persist failed-stream debug artifacts | No |
| `direct_upstream_401` | Upstream 401 propagated to client | No |
| `direct_include_field_is_rejected_fail_closed` | Unsupported `include` field fails closed | No |
| `direct_unknown_top_level_field_rejected` | Unknown top-level fields fail closed | No |
| `direct_unknown_input_item_type_rejected` | Unknown input item types fail closed | No |
| `direct_invalid_raw_input_object_rejected` | Invalid raw input object shape fails closed | No |
| `direct_models_returns_shim_native_catalog` | `/models` returns the generated Codex catalog metadata | No |
| `direct_compact_endpoint_not_implemented` | Unsupported compact endpoint returns not implemented | No |
| `direct_memory_summarize_endpoint_not_implemented` | Unsupported memory summarize endpoint returns not implemented | No |
| `codex_mock_chat_stream_basic` | Full codex exec → shim → mock flow | **Yes** |
| `codex_mock_model_base_instructions_reach_upstream_system_prompt` | Catalog base instructions reach upstream system prompts | **Yes** |
| `codex_mock_request_builder_headers_query_auth` | Validate auth/headers/query params upstream | **Yes** |
| `codex_mock_project_config_trusted_basic` | Project-scoped trusted desktop config works with Codex exec | **Yes** |

### Live Smoke (`codex_live.rs`)

| Test | Description |
|------|------------|
| `live_provider_matrix_no_tool_smoke` | All enabled providers run "return CODEX_SHIM_E2E_OK" |
| `live_provider_tool_smoke` | All enabled providers read a file via shell tool |
| `live_provider_differential_matrix` | Compares enabled providers against an OpenAI baseline across tool, compaction, JSON, and file-edit scenarios |

## Secrets File Format

See `crates/e2e-codex/fixtures/keys.example.toml` for the full template.

Key fields per provider:
- `enabled` — set `true` to include in the matrix
- `profile` — matches provider kind in shim config
- `model` — upstream model name
- `base_url` — upstream API base URL
- `chat_path` — chat completions endpoint (default `/chat/completions`)
- `responses_path` — responses endpoint (default `/responses`)
- `api_key_env` — env var name for the API key
- `api_key` — the actual API key value

Security: API keys are only injected into the shim subprocess environment.
They are never written to config files, stdout, or test failure messages.

## Provider Matrix

| Provider | Profile | Endpoint |
|----------|---------|----------|
| DeepSeek | `deepseek-chat` | `POST /chat/completions` |
| Groq | `groq-chat` | `POST /chat/completions` |
| Fireworks | `fireworks-chat` | `POST /chat/completions` |
| Together | `together-chat` | `POST /chat/completions` |
| OpenRouter Chat | `openrouter-chat` | `POST /chat/completions` |
| OpenRouter Responses | `openrouter-responses` | `POST /responses` |
| Ollama Chat | `ollama-chat` | `POST /chat/completions` |
| Ollama Responses | `ollama-responses` | `POST /responses` |
| vLLM Chat | `vllm-chat` | `POST /chat/completions` |

## CI Integration

```yaml
jobs:
  quality:
    runs-on: ubuntu-latest
    steps:
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
      - run: cargo build -p codex-shim
      - run: cargo test -p e2e-codex --test codex_mock -- --nocapture
        env:
          CODEX_SHIM_BIN: ${{ github.workspace }}/target/debug/codex-shim

  platform-build:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-15-intel, macos-15, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - run: cargo test
      - run: cargo build --release -p codex-shim

  live-provider-smoke:
    if: github.event_name == 'workflow_dispatch'
    runs-on: ubuntu-latest
    steps:
      - run: |
          cargo test -p e2e-codex --test codex_live live_provider_tool_smoke -- --ignored --nocapture
        env:
          CODEX_SHIM_E2E_KEYS: secrets/e2e.providers.toml

  live-differential-matrix:
    if: github.event_name == 'workflow_dispatch'
    continue-on-error: true
    runs-on: ubuntu-latest
    steps:
      - run: |
          cargo test -p e2e-codex --test codex_live live_provider_differential_matrix -- --ignored --nocapture
        env:
          CODEX_SHIM_E2E_OPENAI_MODEL: gpt-5.4-mini
          CODEX_SHIM_E2E_KEYS: secrets/e2e.providers.toml
```

Real API keys should not be stored in CI workspace. Use CI secrets to
generate a temporary `e2e.providers.toml` and delete it after the test run.
The differential matrix should stay manual and non-blocking because live
providers can differ in latency, tool obedience, and compaction behavior.
