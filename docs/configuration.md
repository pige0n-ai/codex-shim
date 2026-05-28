# Configuration Reference

Everything `config.yaml` controls, from minimal to full. For CLI command
details, see [cli.md](cli.md). For desktop app setup, see [desktop.md](desktop.md).

## Minimal Config

A working config needs only three sections. Use `codex-shim setup` to generate
one interactively, or write it by hand:

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

`provider.kind` picks from [27 built-in profiles](provider-compatibility.md).
All other fields (`server.listen`, `reasoning.*`, `sampling.*`, `state.*`,
`logging.*`) use sensible defaults from the chosen profile.

Validate and install:

```bash
./codex-shim validate
export DEEPSEEK_API_KEY="sk-..."
./codex-shim integrate --start
```

## Provider Profiles

`profile_config` is the runtime adapter preset — it controls **how** the shim
talks to the upstream (endpoint mode, reasoning policy, tool policy, state
policy). It is not the model catalog itself.

| Field | Purpose |
|-------|---------|
| `provider.kind` | Legacy shortcut. When both are set, `profile_config.profile` wins. |
| `provider.profile_config.profile` | Real source of truth for shim behavior |
| `provider.profile_config.capabilities` | Override individual capability flags |
| `provider.profile_config.extra_body` | Inject fields into upstream request bodies |

### Capability Overrides

```yaml
provider:
  profile_config:
    profile: deepseek-chat
    capabilities:
      supports_hosted_web_search: false
      supports_streaming_usage: true
    extra_body:
      enable_thinking: true
```

### `kind` vs `profile_config` Precedence

If both are set but differ, `validate` will warn. In `setup`-generated configs,
only `profile_config.profile` is set. Prefer `profile_config.profile`.

## Model Catalog

`models.catalog` is the source of truth for `/models`. Tells Codex what models
exist, their capabilities, and context windows.

### Required per entry

- `slug` — the model name Codex uses in requests
- `context_window` — token limit. Supports suffixes: `128K` (131072),
  `1M` (1048576), `1m` (1000000). Lowercase = decimal, uppercase = binary.

### Commonly set per entry

- `display_name`, `description`, `priority`
- `reasoning_levels` — e.g. `[high, xhigh]`. Defaults from profile capabilities.
- `tool_calling`, `vision` — default from profile capabilities.
- `supports_search_tool` — must be `true` for hosted web search profiles.

### Alignment Rule

These three must agree:
1. Codex `config.toml` → `model`
2. Shim YAML → `models.default`
3. Shim YAML → at least one `models.catalog[*].slug`

## Upstream Configuration

```yaml
upstream:
  base_url: "https://api.deepseek.com"
  chat_path: "/chat/completions"     # default
  responses_path: "/responses"       # default
  api_key_env: "DEEPSEEK_API_KEY"
  requires_openai_auth: false        # skip adapter-side Bearer injection
```

For auth via external command:

```yaml
upstream:
  auth_command:
    command: "/usr/local/bin/fetch-token"
    args: ["--audience", "codex"]
```

### Retry Boundaries

```yaml
upstream:
  max_retries: 2
  stream_max_retries: 2
  downstream_heartbeat_seconds: 30
```

`max_retries` covers ordinary upstream requests and non-streaming chat calls.
`stream_max_retries` covers streaming chat-completions requests only until the
shim has started relaying SSE to Codex. It defaults to `max_retries` when
omitted.

After SSE is already being relayed, codex-shim does not retry or resume a
partial upstream stream. It emits `error` plus `response.failed`; Codex then
uses its native turn-level stream retry to rebuild and re-run the sampling
request from the conversation history it has already committed.

`downstream_heartbeat_seconds` sends a lightweight `response.in_progress` event
when upstream SSE is active but the shim has not emitted a Responses event for
that many seconds. This protects Codex from `idle timeout waiting for SSE` during
reasoning-only chunks, usage-only chunks, and accumulated custom tool arguments.
Set it to `0` to disable the heartbeat.

## Server & Listen Address

```yaml
server:
  listen: "127.0.0.1:8787"   # default
  base_path: "/v1"            # required, do not change
```

`server.listen` controls where the shim binds. When running `integrate`, it is
read from config and used to construct the `base_url` in Codex `config.toml`
(`http://{listen}/v1`). The `setup` wizard asks for this value during
configuration.

## Reasoning

```yaml
reasoning:
  enabled: false     # runtime default
  effort: high       # xhigh, high, medium, low
```

- `reasoning.enabled` controls runtime shim defaults
- `models.catalog[*].reasoning_levels` tells Codex what the model supports

They can differ: a model can be advertised as reasoning-capable while the shim
defaults to disabled.

When `models.catalog[*].base_instructions` is omitted, the generated Codex
model catalog now includes a small Codex coding-agent default so custom upstream
models receive a self-definition and basic tool-use guidance. Set
`base_instructions: ""` explicitly only when you intentionally want Codex to
send empty base instructions for that model.

## Sampling

```yaml
sampling:
  temperature: null  # omit when unset; valid range 0.0..2.0
  top_p: null        # omit when unset; valid range 0.0..1.0
```

These are optional defaults for upstream `/chat/completions` requests. If Codex
already sends `temperature` or `top_p`, that request value is preserved. If the
config field is `null` or omitted, codex-shim leaves it out of the upstream
request body. Provider-specific `pre_send` rules still run last, so profiles
that must remove sampling while reasoning/thinking is enabled continue to do so.

For DeepSeek chat profiles, `reasoning.enabled` controls the typed
`thinking.type` field. `reasoning.enabled: true` sends
`thinking: {type: enabled}` and removes sampling parameters before the upstream
request. `reasoning.enabled: false` sends `thinking: {type: disabled}`, allowing
sampling defaults such as `temperature: 0.0` to remain effective. Do not put a
DeepSeek `thinking` field under `provider.profile_config.extra_body`; the config
validator rejects that shape so invalid OpenAI-compatible payloads fail early.

## Chat Adapter Boundaries

For Chat Completions upstreams, `function_call_output` text is sent as a
standard `role: tool` message. If a tool output contains `input_image` parts,
codex-shim keeps the tool message textual and appends one synthetic `role: user`
multimodal message per image. This is the most compatible shape for
OpenAI-compatible `/chat/completions` providers, whose support for multimodal
`role: tool` messages is inconsistent. Image URLs must still be reachable by the
upstream provider; local paths are not uploaded by the shim.

When a streaming upstream response fails after HTTP 200, failed debug artifacts
include relay diagnostics under `upstream_error`: HTTP status/version, redacted
response headers, reqwest body-error classification, the error source chain, and
the tail of raw upstream SSE data. These fields are intended for distinguishing
provider/body transport failures from mapper failures.

## Other Blocks

```yaml
state:
  backend: memory             # memory/ram, or sqlite when the binary has sqlite support
  ttl_seconds: 86400          # runtime response state for continuation
  debug_artifact_ttl_seconds: 600  # raw debug artifacts; defaults to 10 minutes
  failed_debug_artifact_ttl_seconds: 0  # optional; 0 keeps failed artifacts indefinitely
  sqlite_path: ~/.codex-shim/store.db  # optional; used only with backend: sqlite

logging:
  level: info                 # debug, info, warn, error
```

Defaults work for most setups.
For long benchmark runs, prefer `backend: sqlite`; raw request/SSE debug artifacts
are kept separately from continuation state and expire after
`debug_artifact_ttl_seconds`. Set `failed_debug_artifact_ttl_seconds` to a longer
value when failed upstream/tool/stream attempts need to remain auditable after
successful requests have expired; `0` means no automatic expiry for failed debug
artifacts.

## Validation

```bash
./codex-shim validate                          # syntax + logic
./codex-shim validate --config path/to/config.yaml
./codex-shim validate --check-upstream         # also probe upstream connectivity
```

Checks: YAML syntax, catalog populated, `models.default` resolves, `base_path`
is `/v1`, no legacy `features.*`, `kind`/`profile_config` consistency,
reasoning consistency. Non-zero exit on errors — CI-friendly.

## Codex Integration

Generate Codex startup files from your shim config:

```bash
codex-shim integrate --config ~/.codex-shim/config.yaml
```

Writes `$CODEX_HOME/model-catalog-shim.json` and updates `$CODEX_HOME/config.toml`
(with `.bak.0`–`.bak.3` rolling backups). The `base_url` in the resulting
`config.toml` is constructed from `server.listen`.

Resulting Codex `config.toml` shape:

```toml
model_provider = "codex_shim"
model = "deepseek-v4-pro"
model_catalog_json = "/path/to/$CODEX_HOME/model-catalog-shim.json"
web_search = "disabled"

[model_providers.codex_shim]
name = "codex-shim"
base_url = "http://127.0.0.1:8787/v1"
wire_api = "responses"
supports_websockets = false
```

Key rules:
- `wire_api = "responses"` is the only supported protocol.
- `supports_websockets = false` is required (HTTP/SSE only).
- `web_search = "disabled"` for Chat upstreams. Native Responses profiles may use `cached`/`live`.
- Omit `env_key` for local loopback. Add it only for bearer-protected or remote shim gateways.

## Desktop Project Config

For the Codex desktop app, prefer project-scoped install:

```bash
codex-shim integrate \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo \
  --trust-project
```

Validates with:

```bash
codex-shim doctor desktop \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo
```

See [desktop.md](desktop.md) for the full desktop support contract.

## Precedence Rules (Summary)

1. `profile_config.profile` > `provider.kind` for runtime behavior.
2. Explicit `models.catalog[*]` fields > profile-derived defaults.
3. Codex `model` must exist in the shim catalog.
4. `reasoning.enabled` (runtime) is independent of catalog `reasoning_levels` (capability declaration).

## Related Files

- [CLI Reference](cli.md)
- [Provider Compatibility](provider-compatibility.md)
- [Desktop Support](desktop.md)
- [E2E Testing](e2e.md)
- `examples/all-options.yaml` — every config key with comments
