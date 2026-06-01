# codex-shim

[中文说明](README.zh-CN.md)

`codex-shim` is a local adapter for using non-OpenAI or self-hosted models with
[Codex](https://developers.openai.com/codex) custom model providers. Codex talks
to one familiar `/v1/responses` endpoint; the shim translates that traffic to an
upstream Chat Completions endpoint or passes it through to a native Responses
endpoint.

Use it when you want Codex CLI or Codex Desktop to work with DeepSeek,
OpenRouter, xAI, Gemini, Groq, Ollama, vLLM, llama.cpp, SGLang, or another
OpenAI-compatible backend without rewriting Codex-side configuration by hand.

## Architecture

```text
Codex (wire_api = "responses") -> codex-shim /v1/responses
                                      |-> NativeResponses     -> upstream /v1/responses
                                      |-> StatelessResponses  -> upstream /v1/responses
                                      |                         with history materialized
                                      `-> ChatCompletionsShim -> upstream /v1/chat/completions
```

Codex always sees the Responses API. Chat Completions is an adapter-internal
upstream protocol only.

## Platform Support

Release binaries are built for Linux `x86_64` (musl static), macOS
`x86_64`/`aarch64`, and Windows `x86_64`.

Codex CLI works across all three platforms: start the shim, run `integrate`,
then launch `codex` normally.

Codex Desktop is macOS-first. The macOS model picker reads
`model_catalog_json` and lists your models. On Windows, the desktop app may show
only "Custom" in the model picker even when the catalog has multiple entries;
Codex still uses the top-level `model` value from `config.toml`.

If Codex Desktop is configured to run its agent in WSL, requests are routed from
inside the WSL network namespace. A shim bound only to `127.0.0.1` on the
Windows host is not reachable from there. Bind the shim to `0.0.0.0` or the WSL
host IP, and point `config.toml` at that address. In this setup, run the Windows
binary to install the Windows Codex config, and run the Linux binary inside WSL
with the same shim config to bind the listen address.

## Quick Start

Download the raw binary for your platform from a release. `refs.zip` contains
the reference files (`examples/`, `README.md`, and `LICENSE`) if you want them
next to the binary.

First run:

```bash
./codex-shim setup --yolo
```

The wizard walks through provider, model, API key environment variable, and
listen address, then writes the shim config, installs the Codex provider files,
and starts the server. If you run `codex-shim` with no subcommand and no default
config exists, it opens this first-run flow automatically.

Step by step:

```bash
./codex-shim setup              # write config interactively
./codex-shim setup --integrate  # setup + install Codex files
export DEEPSEEK_API_KEY="sk-..."
./codex-shim integrate --start --config ~/.codex-shim/config.yaml
```

Build from source:

```bash
cargo build --release -p codex-shim
./target/release/codex-shim --config examples/deepseek-chat/config.yaml
```

## Mental Model

There are three pieces to keep aligned:

| Layer | File | Purpose |
| --- | --- | --- |
| Codex config | `$CODEX_HOME/config.toml` | Tells Codex how to reach the shim |
| Shim config | `~/.codex-shim/config.yaml` | Tells the shim how to reach the upstream provider |
| Model catalog | `models.catalog` in shim YAML | Tells Codex what model metadata and tools to use |

These values should agree: Codex `model`, shim `models.default`, and at least
one `models.catalog[*].slug`.

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

See [docs/cli.md](docs/cli.md) for the full command reference.

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
      apply_patch_tool_type: freeform
```

For the full reference, read [docs/configuration.md](docs/configuration.md).
For every supported key with comments, see
[examples/all-options.yaml](examples/all-options.yaml).

## Authentication

There are two independent auth layers:

1. Codex -> shim: optional bearer auth for the local hop, configured with
   `accepted_bearer_tokens`.
2. Shim -> upstream: your real provider credential, usually referenced through
   `upstream.api_key_env`, such as `DEEPSEEK_API_KEY`.

The shim does not forward its own local bearer token to upstream providers.

## Provider Profiles

`codex-shim` bundles 27 profiles covering hosted APIs (DeepSeek, OpenRouter,
xAI, Groq, Gemini, and others), local/self-hosted servers (Ollama, vLLM,
llama.cpp, SGLang), and generic OpenAI-compatible upstreams.

See [docs/provider-compatibility.md](docs/provider-compatibility.md) for the
matrix and provider-specific notes.

## Runtime Behavior

- Chat Completions streams are retried only before downstream SSE has started.
  Once Codex has received events, mid-stream upstream failures become
  `response.failed` plus debug artifacts so Codex can use its own turn-level
  retry.
- `upstream.downstream_heartbeat_seconds` emits lightweight
  `response.in_progress` events during long reasoning-only, usage-only, or
  accumulated custom-tool chunks. Set it to `0` to disable.
- Failed raw request/SSE debug artifacts default to no automatic expiry with
  `state.failed_debug_artifact_ttl_seconds: 0`; successful artifacts still use
  `state.debug_artifact_ttl_seconds`.
- Explicit catalog entries should set `apply_patch_tool_type: freeform` when
  Codex should expose patch editing. Chat upstreams can optionally use
  `apply_patch_upstream_tool_type: structured`; structured calls must include
  `raw_patch`.
- Chat upstreams receive multimodal tool outputs as textual `role: tool`
  acknowledgements plus synthetic user image messages for better provider
  compatibility.

## Codex Desktop

For desktop project setup:

```bash
codex-shim integrate \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo \
  --trust-project

codex-shim doctor desktop \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo
```

Desktop support is intentionally conservative: one trusted project, one stable
`model_provider = "codex_shim"`, and one project-scoped model catalog. Details
are in [docs/desktop.md](docs/desktop.md).

## Safety Notes

Back up `$CODEX_HOME/config.toml` before first use. codex-shim keeps up to four
rolling backups (`.bak.0` to `.bak.3`) when it updates the file, but your own
copy is still worth having.

Thread history is tied to Codex's `model_provider` key. Changing
`model_provider` can hide existing threads in the UI. They are not deleted:
restore the previous config or switch back to the previous provider key to see
them again.

Chat Completions compatibility depends on the upstream provider's tool and
streaming behavior. If a provider is close but not quite compatible, please
[file an issue](https://github.com/pige0n-ai/codex-shim/issues).

## Tests

```bash
cargo test                                               # unit + integration
cargo test -p e2e-codex --test codex_mock                # mock E2E (offline)
cargo test -p e2e-codex --test codex_mock -- --ignored   # + Codex blackbox

CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live -- --ignored --nocapture
```

See [docs/e2e.md](docs/e2e.md).

## License

MIT
