# codex-shim

Local adapter that exposes a `/v1/responses` API compatible with
[Codex](https://developers.openai.com/codex) custom model providers,
and maps requests to upstream Chat Completions or native Responses endpoints.

## Architecture

```
Codex (wire_api="responses") → codex-shim /v1/responses
                                   ├── NativeResponses     → upstream /v1/responses
                                   ├── StatelessResponses  → upstream /v1/responses (history materialization)
                                   └── ChatCompletionsShim → upstream /v1/chat/completions
```

Codex **always** sees the Responses API. Chat Completions is an
**adapter-internal** upstream protocol only.

## Supported Platforms

Linux `x86_64` (musl static), macOS `x86_64`/`aarch64`, Windows `x86_64`.

## Limitations

- **Back up your Codex config** by keeping a copy of `$CODEX_HOME/config.toml` 
  before first use.

- **Codex CLI** works on all three platforms. Start the shim, run `integrate`,
  then launch `codex` as normal.

- **Codex Desktop** — macOS is better-supported: the model picker reads
  `model_catalog_json` and lists your models. Windows has two known issues:
  - When Codex Desktop uses a **WSL environment**, requests are routed through
    WSL's network namespace. A shim bound to `127.0.0.1` on the Windows host
    will **not** be reachable. Bind to `0.0.0.0` or the WSL host IP instead,
    and use that address in `config.toml`.
  - The chat model picker only shows "Custom" regardless of
    `model_catalog_json` contents, and does not allow switching between
    multiple catalog entries. Codex uses, though not visible, whichever model 
    is set in the top-level `model` field of `config.toml`.

- **Thread history is tied to `model_provider`.** Changing the
  `model_provider` key in `config.toml` (e.g. switching away from
  `codex_shim`) will hide existing threads in Codex. They are not lost —
  restore the `config.toml`, or switch back to the previous `model_provider` 
  to recover them.

- **Chat Completions shim** translates Codex Responses requests to upstream
  `/chat/completions`. Tool call formats and streaming semantics vary across
  providers, so compatibility is not uniform. For provider-specific issues,
  please [file an issue](https://github.com/pige0n-ai/codex-shim/issues).

- **Back up your Codex config** by keeping a copy of `$CODEX_HOME/config.toml` 
  before first use. codex-shim retains up to 4 rolling backups 
  (`.bak.0`–`.bak.3`) when updating `config.toml`, but you should still 
  keep your own.

## Quick Start

### Interactive Setup

```bash
./codex-shim setup --yolo
```

Walks through provider, model, API key, and listen address — then writes config,
installs Codex startup files, and starts the server.

### Step by Step

```bash
./codex-shim setup              # write config interactively
./codex-shim setup --integrate  # setup + install Codex files
export DEEPSEEK_API_KEY="sk-..."
./codex-shim integrate --start --config ~/.codex-shim/config.yaml
```

### Build From Source

```bash
cargo build --release -p codex-shim
./target/release/codex-shim --config examples/deepseek-chat/config.yaml
```

## Key Concepts

| Layer | File | Purpose |
|-------|------|---------|
| Codex config | `$CODEX_HOME/config.toml` | Tells Codex how to reach the shim |
| Shim config | `~/.codex-shim/config.yaml` | Tells the shim how to reach your upstream provider |
| Model catalog | inside shim YAML | Tells the shim what model metadata to advertise to Codex |

Keep these three values aligned: Codex `model`, shim `models.default`, and at least one
`models.catalog[*].slug`.

## CLI

```bash
codex-shim [OPTIONS] [COMMAND]

Commands:
  setup                Interactive setup wizard (recommended)
  integrate            Validate config + install Codex startup catalog
  validate             Check config YAML for correctness
  config-show          Inspect resolved config (summary/yaml/json)
  generate-catalog     Generate a model catalog JSON
  explain-catalog      Explain what a model catalog JSON means to Codex
  probe                Probe an upstream endpoint for capabilities
  doctor               Validate desktop-oriented Codex project wiring

Options:
  -c, --config <PATH>       Path to config YAML (default: ~/.codex-shim/config.yaml)
      --listen <ADDR>       Listen address (default: 127.0.0.1:8787)
```

See [docs/cli.md](docs/cli.md) for full details on every command.

## Two Auth Layers

1. **Codex → shim** (optional): bearer token for the local loopback hop. Only
   needed when protecting the shim with `accepted_bearer_tokens`.
2. **Shim → upstream**: your real API key, set via `upstream.api_key_env` in
   the shim config (e.g. `export DEEPSEEK_API_KEY="sk-..."`).

These tokens are independent — the shim never forwards its own auth to upstreams.

## Provider Profiles

27 built-in profiles covering hosted APIs (DeepSeek, OpenRouter, xAI, Groq, etc.),
local/self-hosted (Ollama, vLLM, llama.cpp, SGLang), and generic OpenAI-compatible
upstreams. See [docs/provider-compatibility.md](docs/provider-compatibility.md).

## Configuration

Minimal config:

```yaml
provider:
  kind: deepseek-chat
  profile_config:
    profile: deepseek-chat

upstream:
  api_key_env: DEEPSEEK_API_KEY

models:
  default: deepseek-v4-pro
  catalog:
    - slug: deepseek-v4-pro
      context_window: 131072
```

For the full reference: [docs/configuration.md](docs/configuration.md).
For desktop app setup: [docs/desktop.md](docs/desktop.md).
For all available options with comments: `examples/all-options.yaml`.

## Tests

```bash
cargo test                                         # unit + integration
cargo test -p e2e-codex --test codex_mock          # mock E2E (offline)
cargo test -p e2e-codex --test codex_mock -- --ignored  # + Codex blackbox

# Live provider smoke (requires API keys)
CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live -- --ignored --nocapture
```

See [docs/e2e.md](docs/e2e.md).

## License

MIT
