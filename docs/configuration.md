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
All other fields (`server.listen`, `reasoning.*`, `state.*`, `logging.*`) use
sensible defaults from the chosen profile.

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

## Other Blocks

```yaml
state:
  backend: sqlite             # default

logging:
  level: info                 # debug, info, warn, error
```

Defaults work for most setups.

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
