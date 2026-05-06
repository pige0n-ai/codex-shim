# codex-shim

Local adapter that exposes a `/v1/responses` API compatible with
[Codex](https://developers.openai.com/codex) custom model providers,
and maps requests to upstream Chat Completions or native Responses endpoints.

## Architecture

```
Codex (wire_api="responses") → codex-shim /v1/responses
                                  ├── NativeResponses   → upstream /v1/responses
                                  ├── StatelessResponses → upstream /v1/responses (history materialization)
                                  └── ChatCompletionsShim → upstream /v1/chat/completions
```

Codex **always** sees the Responses API. Chat Completions is an
**adapter-internal** upstream protocol only.

## Supported Platforms

- Linux `x86_64`
- macOS `x86_64` and `aarch64`
- Windows `x86_64`

Core tests run on Linux, macOS, and Windows in CI. Live provider checks stay
manual because they require real credentials and upstream network access.
Published release archives are built and packaged on native GitHub-hosted
runners for each platform.

## Quick Start

```bash
# Build from source
cargo build --release -p codex-shim

# Run with DeepSeek
export DEEPSEEK_API_KEY="sk-..."
./target/release/codex-shim --config examples/deepseek-chat/config.yaml

# Run with Ollama
./target/release/codex-shim --config examples/ollama-chat/config.yaml
```

Windows PowerShell:

```powershell
cargo build --release -p codex-shim

$env:DEEPSEEK_API_KEY = "sk-..."
.\target\release\codex-shim.exe --config examples\deepseek-chat\config.yaml
```

Published release archives include the platform binary, `README.md`,
`LICENSE`, and the `examples/` directory.

## Codex Provider Config

In `$CODEX_HOME/config.toml`:

```toml
model_provider = "local-shim"
model = "your-model-slug"

# Chat Completions upstreams cannot execute hosted tools.
# Disable web_search so Codex does not send hosted web_search tools.
web_search = "disabled"

[tools]
web_search = false

[features]
web_search_request = false

[model_providers.local-shim]
name = "codex-shim"
base_url = "http://127.0.0.1:8787/v1"
env_key = "LOCAL_SHIM_TOKEN"
wire_api = "responses"
supports_websockets = false  # REQUIRED: shim is HTTP/SSE only
```

> **Codex Configuration Notes**
>
> - `wire_api = "responses"` is required — Codex always sees the Responses API.
> - `supports_websockets = false` is required — shim only supports HTTP/SSE.
> - For Chat Completions upstreams, disable `web_search` as shown above.
>   Without this, Codex may send hosted `web_search` tools, and the shim
>   will correctly reject them with `not_implemented`.
> - If your upstream is a Native Responses provider that supports hosted
>   tools, you may enable `web_search` and remove the `[tools]`/`[features]` blocks.
> - The shim now serves its own `/models` catalog. `model_catalog_json` is
>   optional and best treated as an offline pin or manual override, not the
>   primary discovery path.
> - Default Codex home is usually `~/.codex` on macOS/Linux and
>   `%USERPROFILE%\\.codex` on Windows when `CODEX_HOME` is unset.

See `examples/codex-config/config.toml` for the full annotated example.

## Default Paths

- Shim config default: `~/.codex-shim/config.yaml` on macOS/Linux,
  `%USERPROFILE%\\.codex-shim\\config.yaml` on Windows.
- SQLite state default when `state.backend = "sqlite"` and no explicit
  `sqlite_path` is configured: `~/.codex-shim/store.db` on macOS/Linux,
  `%USERPROFILE%\\.codex-shim\\store.db` on Windows.

## Authentication: Two Layers

Codex-shim sits between Codex and your upstream provider — there are TWO
separate authentication layers:

1. **Codex → codex-shim**: The token Codex sends when calling your local
   adapter. Set `env_key = "LOCAL_SHIM_TOKEN"` in your Codex provider config
   and export it before starting Codex:
   ```bash
   export LOCAL_SHIM_TOKEN="sk-any-value"
   ```
   If `accepted_bearer_tokens` is empty in your shim config, any token passes.

2. **codex-shim → upstream provider**: Your actual API key for DeepSeek,
   OpenRouter, Ollama, etc. This is set via `upstream.api_key_env` in your
   shim config:
   ```bash
   export DEEPSEEK_API_KEY="sk-..."
   ./target/release/codex-shim --config examples/deepseek-chat/config.yaml
   ```
   For proxy/forwarding setups where an external layer handles authentication,
   set `upstream.requires_openai_auth: true` — this disables adapter-side Bearer
   injection. Note: `requires_openai_auth` does NOT fetch or inject an OpenAI
   login token; it only skips adapter Bearer auth.
   Or for command-based auth:
   ```yaml
   upstream:
     auth_command:
       command: "/usr/local/bin/fetch-token"
       args: ["--audience", "codex"]
   ```

These two tokens are independent. `LOCAL_SHIM_TOKEN` is never forwarded
to upstream providers.

## Built-in Provider Profiles

| Profile | Upstream | Endpoint | Reasoning |
|---------|----------|----------|-----------|
| `deepseek-chat` | api.deepseek.com | Chat Shim | `reasoning_content` |
| `sglang-chat` | localhost:30000/v1 | Chat Shim | `reasoning_content` + `chat_template_kwargs` |
| `vllm-responses` | localhost:8000/v1 | Native Responses | native reasoning item |
| `vllm-chat` | localhost:8000/v1 | Chat Shim | `reasoning_content` |
| `ollama-responses` | localhost:11434/v1 | Stateless Responses | native reasoning item |
| `ollama-chat` | localhost:11434/v1 | Chat Shim | generic |
| `llamacpp-responses` | localhost:8080/v1 | Native Responses | native |
| `llamacpp-chat` | localhost:8080/v1 | Chat Shim | generic |
| `openrouter-responses` | openrouter.ai/api/v1 | Stateless Responses | `reasoning_details` |
| `openrouter-chat` | openrouter.ai/api/v1 | Chat Shim | `reasoning_details` |
| `alibaba-responses` | dashscope.aliyuncs.com | Native Responses (stateful) | `enable_thinking` |
| `alibaba-chat` | dashscope.aliyuncs.com | Chat Shim | `enable_thinking` |
| `groq-chat` | api.groq.com | Chat Shim | generic |
| `together-chat` | api.together.xyz | Chat Shim | generic |
| `fireworks-chat` | api.fireworks.ai | Chat Shim | generic |
| `generic-chat` | (configurable) | Chat Shim | generic |

Detailed config per provider in `examples/<profile>/config.yaml`.

## Limitations

1. **Hosted tools** (`web_search`, `file_search`, `code_interpreter`,
   `computer_use`, `mcp`) are **not supported** through Chat Completions
   upstreams. Codex requests using these tools will receive a clear error.
   Enable them only with native Responses providers that explicitly support them.

2. **`input_file` / multipart** is not implemented. File uploads and
   server-side file retrieval return an error.

3. **`previous_response_id`** is simulated via local store for non-stateful
   upstreams. This is not equivalent to native upstream conversation state.

4. **WebSocket transport** is not supported. Codex provider configs must set
   `supports_websockets = false`.

5. **`model_verbosity`, `conversation`, `background`, `truncation`,
   `max_tool_calls`** are not implemented and return errors.

6. ChatGPT credits, fast mode, Codex app-server features, approval flows,
   and memory summarization are Codex-level capabilities that do not
   transfer to external providers through this adapter.

7. **`/responses/compact` and `/memories/trace_summarize`** are explicitly
   not implemented. In custom-provider mode, Codex uses local compaction.
   The shim does not attempt to emulate OpenAI server-side compaction or
   memory summarization semantics.

8. **Shim config is strict**:
   - `server.base_path` must remain `/v1`
   - `features.*` in shim YAML is rejected at startup
   - `models.catalog` must be present so `/models` can be served natively

## CLI

```bash
codex-shim [OPTIONS] [COMMAND]

Commands:
  generate-catalog  Generate a model catalog JSON for a provider profile
  explain-catalog   Explain what a model catalog JSON means to Codex
  probe             Probe an upstream endpoint and report detected capabilities

Options:
  -c, --config <CONFIG>              Path to config YAML file
      --listen <LISTEN>              Listen address (default 127.0.0.1:8787)
      --provider <PROVIDER>          Provider kind
      --upstream-base <URL>          Upstream base URL
      --upstream-key-env <ENV>       API key env var name
      --model <MODEL>                Default model
```

## Configuration Reference

See `examples/all-options.yaml` for every available config key with comments.

## Tests

```bash
cargo test                                   # unit + integration tests
cargo test -p e2e-codex --test codex_mock    # mock E2E (offline, auto-builds shim)
cargo test -p e2e-codex --test codex_mock -- --ignored  # + Codex blackbox tests

# Live provider smoke across all enabled providers (requires API keys)
CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live -- --ignored --nocapture

# Live differential checks (manual canary, not a CI gate)
# Preferred: direct API key baseline
OPENAI_API_KEY=sk-... \
CODEX_SHIM_E2E_OPENAI_MODEL=gpt-5.4-mini \
CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live live_provider_differential_matrix -- --ignored --nocapture

# Fallback: reuse an existing Codex auth cache when no API key is available
CODEX_SHIM_E2E_OPENAI_AUTH_JSON="${CODEX_HOME:-$HOME/.codex}/auth.json" \
CODEX_SHIM_E2E_OPENAI_MODEL=gpt-5.4-mini \
CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live live_provider_differential_matrix -- --ignored --nocapture
```

See `docs/e2e.md` for full E2E testing documentation.

## License

MIT
