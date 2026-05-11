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

The intended path is: download a release archive, unpack it, start the shim
with one bundled example config, then let `codex-shim` inject the matching
Codex startup catalog and provider config for you.

If you want the fastest post-download setup path, or a full explanation of how
`profile_config`, `models.catalog`, and Codex's `config.toml` fit together, see
[docs/configuration.md](/home/vivec/codex-shim/docs/configuration.md).
For the Codex desktop app contract, including trusted project setup and
history/resume boundaries, see
[docs/desktop.md](/home/vivec/codex-shim/docs/desktop.md).

### Use A Release Binary

1. Download the archive for your platform from
   [Releases](https://github.com/pige0n-ai/codex-shim/releases/latest).
2. Unpack it.
3. Open the unpacked directory; it already contains the binary, `examples/`,
   `README.md`, and `LICENSE`.

Linux / macOS:

```bash
tar -xzf codex-shim-<version>-<target>.tar.gz
cd codex-shim-<version>-<target>

export DEEPSEEK_API_KEY="sk-..."
./codex-shim install-codex-config --config examples/deepseek-chat/config.yaml
./codex-shim --config examples/deepseek-chat/config.yaml
```

Windows PowerShell:

```powershell
Expand-Archive .\codex-shim-<version>-x86_64-pc-windows-msvc.zip
cd .\codex-shim-<version>-x86_64-pc-windows-msvc

$env:DEEPSEEK_API_KEY = "sk-..."
.\codex-shim.exe install-codex-config --config .\examples\deepseek-chat\config.yaml
.\codex-shim.exe --config .\examples\deepseek-chat\config.yaml
```

The install command writes:

- `$CODEX_HOME/model-catalog-shim.json`
- `$CODEX_HOME/config.toml`
- `$CODEX_HOME/config.toml.bak.0` ... `.bak.3` rolling backups when `config.toml` already exists

After that, restart Codex and use it normally:

```bash
codex
# or
codex exec "Explain this repository"
```

### Build From Source

```bash
cargo build --release -p codex-shim
export DEEPSEEK_API_KEY="sk-..."
./target/release/codex-shim install-codex-config --config examples/deepseek-chat/config.yaml
./target/release/codex-shim --config examples/deepseek-chat/config.yaml
```

## GUI Beta

A lightweight desktop GUI shell now lives under
[gui/README.md](/home/vivec/codex-shim/gui/README.md).

Its scope is intentionally narrow:

- runtime status
- token usage curve
- live logs
- a gear-driven configuration drawer for shim YAML and Codex TOML integration

The GUI uses `Tauri v2` with a static HTML/CSS/JS frontend. It does not need a
Node-based frontend build step, but Linux builds still require the usual
GTK/WebKit system packages that Tauri depends on.

### Codex Desktop App

A lightweight desktop GUI shell now lives under
[gui/README.md](/home/vivec/codex-shim/gui/README.md).

Its scope is intentionally narrow:

- runtime status
- token usage curve
- live logs
- a gear-driven configuration drawer for shim YAML and Codex TOML integration

The GUI uses `Tauri v2` with a static HTML/CSS/JS frontend. It does not need a
Node-based frontend build step, but Linux builds still require the usual
GTK/WebKit system packages that Tauri depends on.

## Codex Provider Config

The simplest path is not to edit this by hand. Run:

```bash
codex-shim install-codex-config --config ~/.codex-shim/config.yaml
```

That updates `$CODEX_HOME/config.toml` automatically.

If you want to inspect or hand-edit the result, the important shape is:

```toml
model_provider = "codex_shim"
model = "deepseek-v4-pro"
model_catalog_json = "/absolute/path/to/$CODEX_HOME/model-catalog-shim.json"

# Most shim profiles ultimately target Chat Completions upstreams, so hosted
# web search should start disabled. If you use a native Responses provider that
# really supports hosted search through the shim, switch this to "cached" or
# "live" and advertise that support in the shim model catalog.
web_search = "disabled"

[model_providers.codex_shim]
name = "codex-shim"
base_url = "http://127.0.0.1:8787/v1"
wire_api = "responses"        # explicit for clarity; this is also the default
supports_websockets = false   # REQUIRED: codex-shim is HTTP/SSE only
```

> **Codex Configuration Notes**
>
> - `model_provider` must reference a custom entry under `model_providers`.
> - The official Codex config docs reserve `openai`, `ollama`, `lmstudio`,
>   and `amazon-bedrock` as built-in provider IDs, so use a different custom ID.
> - `wire_api = "responses"` is the only supported provider protocol in Codex.
> - `supports_websockets = false` is required — shim only supports HTTP/SSE.
> - Top-level `web_search` is the current Codex setting. The older
>   `features.web_search*` toggles are deprecated in the official config reference.
> - For Chat Completions upstreams, keep `web_search = "disabled"` as shown above.
>   Without this, Codex may send hosted `web_search` tools that the shim
>   correctly rejects.
> - If your upstream is a native/stateless Responses provider that supports hosted
>   search through the shim, you may change `web_search` to `cached` or `live`.
> - Current Codex still loads startup metadata from top-level `model_catalog_json`.
>   Without it, custom models may fall back to generic metadata and may not show up
>   in `/model`, even though the shim serves `/v1/models` at runtime.
> - `env_key` for Codex → shim auth is optional. For a local loopback server on
>   `127.0.0.1`, the default installer omits it to reduce setup friction.
> - Add `--env-key LOCAL_SHIM_TOKEN` only when you want Codex to authenticate to a
>   remote/shared shim gateway or a loopback shim protected by bearer auth.
> - Default Codex home is usually `~/.codex` on macOS/Linux and
>   `%USERPROFILE%\\.codex` on Windows when `CODEX_HOME` is unset.

See `examples/codex-config/config.toml` for the full annotated example.
For the complete end-to-end configuration flow, see
[docs/configuration.md](/home/vivec/codex-shim/docs/configuration.md).

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
   adapter. This is optional for a local loopback shim. Add it only when the
   shim itself is protected by bearer auth or exposed as a remote gateway:
   ```bash
   codex-shim install-codex-config --config ~/.codex-shim/config.yaml --env-key LOCAL_SHIM_TOKEN
   export LOCAL_SHIM_TOKEN="sk-any-value"
   ```
   If `accepted_bearer_tokens` is empty in your shim config, requests can pass
   without any bearer token at all.

2. **codex-shim → upstream provider**: Your actual API key for DeepSeek,
   OpenRouter, Ollama, etc. This is set via `upstream.api_key_env` in your
   shim config:
   ```bash
   export DEEPSEEK_API_KEY="sk-..."
   ./codex-shim --config examples/deepseek-chat/config.yaml
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

These two tokens are independent. If you do use `LOCAL_SHIM_TOKEN`, it is never
forwarded to upstream providers.

## Built-in Provider Profiles

Bundled hosted-provider presets now include:

- chat-only: `deepseek-chat`, `zai-chat`, `moonshot-chat`, `minimax-chat`, `gemini-chat`, `vertex-chat`
- chat + responses: `alibaba-chat`, `alibaba-responses`, `fireworks-chat`, `fireworks-responses`, `xai-chat`, `xai-responses`, `bedrock-chat`, `bedrock-responses`
- router/self-hosted variants: `openrouter-chat`, `openrouter-responses`, `groq-chat`, `groq-responses`, `together-chat`, `ollama-chat`, `ollama-responses`, `llamacpp-chat`, `llamacpp-responses`, `vllm-chat`, `vllm-responses`, `sglang-chat`
- generic fallback: `generic-chat`

For the audited endpoint support, streaming-usage notes, statefulness, auth
shape, and official evidence links, see
[docs/provider-compatibility.md](/home/vivec/codex-shim/docs/provider-compatibility.md).

## Example Configs

Every bundled `examples/<profile>/config.yaml` is meant to be runnable after you:

- set the required API key environment variable if the provider is remote
- replace any remaining local/self-hosted placeholder model with the actual
  model route exposed by your deployment
- keep `models.default` and `models.catalog[0].slug` in sync

If you only want one path to copy first, start with:

- `examples/deepseek-chat/config.yaml` for a hosted Chat Completions provider
- `examples/openrouter-responses/config.yaml` for a stateless Responses provider
- `examples/ollama-chat/config.yaml` or `examples/ollama-responses/config.yaml` for local OSS
- `examples/gemini-chat/config.yaml` or `examples/xai-responses/config.yaml`
  for newly added hosted providers

## Configuration Summary

There are three separate configuration layers:

1. Codex `config.toml`: tells Codex how to call the local shim
2. shim `config.yaml`: tells the shim how to call the upstream provider
3. shim `models.catalog`: tells the shim what model metadata to advertise back
   to Codex through `/models`

Keep these values aligned:

- Codex `config.toml`: top-level `model`
- shim `config.yaml`: `models.default`
- shim `config.yaml`: at least one `models.catalog[*].slug`

Important distinctions:

- `profile_config` controls shim runtime behavior toward the upstream provider
- `models.catalog` controls the model metadata Codex sees
- `reasoning.enabled` is a runtime/default shim setting
- `models.catalog[*].reasoning_levels` is a model capability declaration to Codex

For the full walkthrough, field guide, and precedence rules, see
[docs/configuration.md](/home/vivec/codex-shim/docs/configuration.md).

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

9. **Desktop support is project-scoped**:
   - use project `.codex/config.toml` plus a trusted project entry for the Codex desktop app
   - shim-managed history/resume guarantees only apply to threads created under that stable `codex_shim` provider identity
   - old non-shim desktop threads remain a gated compatibility case; see [docs/desktop.md](/home/vivec/codex-shim/docs/desktop.md)

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
