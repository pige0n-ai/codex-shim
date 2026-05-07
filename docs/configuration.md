# Configuration Guide

Start here if you already have a `codex-shim` release archive and want the
fastest path to a working setup.

The short version is:

1. Copy one bundled shim example instead of writing `config.yaml` from scratch.
2. Keep one model slug aligned across Codex and the shim.
3. Do not touch `profile_config` unless you are intentionally overriding a
   built-in provider preset.

The sections below start with the 5-minute setup flow, then explain the config
layers and precedence rules in more detail.

## First-Time Setup In 5 Minutes

This example uses `examples/deepseek-chat/config.yaml`, but the same flow works
for other bundled profiles.

### 1. Start from a bundled example

Use the release archive you unpacked earlier. Do not write a fresh shim YAML
from scratch for your first setup.

Pick one of these starting points:

- hosted Chat Completions provider: `examples/deepseek-chat/config.yaml`
- stateless Responses provider: `examples/openrouter-responses/config.yaml`
- local OSS provider: `examples/ollama-chat/config.yaml`

For the full built-in profile matrix, including which providers have native
`/responses`, how streaming usage behaves, and which examples use
`auth_command`, see
[docs/provider-compatibility.md](/home/vivec/codex-shim/docs/provider-compatibility.md).

### 2. Copy the shim config to its normal location

macOS / Linux:

```bash
mkdir -p ~/.codex-shim
cp examples/deepseek-chat/config.yaml ~/.codex-shim/config.yaml
```

Windows PowerShell:

```powershell
New-Item -ItemType Directory -Force "$HOME\.codex-shim" | Out-Null
Copy-Item .\examples\deepseek-chat\config.yaml "$HOME\.codex-shim\config.yaml"
```

You can also keep the file inside the unpacked release directory and pass
`--config /path/to/config.yaml` explicitly. Copying it to the default path is
usually simpler for day-to-day use.

### 3. Edit only the minimum fields

For a first working setup, only change:

- `upstream.api_key_env`
- `models.default`
- `models.catalog[*].slug`

Keep `models.default` equal to the catalog slug.

For example:

```yaml
upstream:
  api_key_env: "DEEPSEEK_API_KEY"

models:
  default: "deepseek-v4-pro"
  catalog:
    - slug: "deepseek-v4-pro"
      context_window: 131072
```

Do not change `provider.profile_config.profile` unless you are deliberately
switching to a different provider preset.

### 4. Export the upstream API key

For a local loopback shim, you usually only need the upstream provider key.

macOS / Linux:

```bash
export DEEPSEEK_API_KEY="sk-..."
```

Windows PowerShell:

```powershell
$env:DEEPSEEK_API_KEY = "sk-..."
```

If you later decide to protect Codex → shim with bearer auth, reinstall with:

```bash
./codex-shim install-codex-config --config /absolute/path/to/config.yaml --env-key LOCAL_SHIM_TOKEN
```

### 5. Start the shim

If you copied the config to the default path:

```bash
./codex-shim
```

If you kept the config somewhere else:

```bash
./codex-shim --config /absolute/path/to/config.yaml
```

The default config path is:

- macOS / Linux: `~/.codex-shim/config.yaml`
- Windows: `%USERPROFILE%\.codex-shim\config.yaml`

### 6. Install the matching Codex startup config

Run:

```bash
./codex-shim install-codex-config --config /absolute/path/to/config.yaml
```

That command writes:

- `$CODEX_HOME/codex-shim/model-catalog.json`
- `$CODEX_HOME/config.toml`
- `$CODEX_HOME/config.toml.bak.0` ... `.bak.3` rolling backups when `config.toml` already exists

For a first setup, keep the Codex `model` equal to the shim `models.default`.
You only need `--model ...` when the same shim YAML advertises more than one
catalog entry and you want a different default.

### 7. Run Codex

```bash
codex
```

If requests reach the shim but fail upstream, the first things to check are:

- the upstream API key env var name in `upstream.api_key_env`
- the upstream model slug in `models.default` and `models.catalog[*].slug`
- the Codex `model` value in `$CODEX_HOME/config.toml`

## Before You Customize Anything

These defaults are intentional:

- keep `wire_api = "responses"` in Codex
- keep `supports_websockets = false`
- keep top-level Codex `web_search = "disabled"` unless your chosen shim
  profile really supports hosted search through a Responses upstream
- leave `profile_config.capabilities` alone unless you are correcting a known
  provider quirk
- leave `reasoning.enabled` alone until the basic request path works

Once the basic path works, move on to the reference sections below.

## The Three Configuration Layers

### 1. Codex config: `$CODEX_HOME/config.toml`

This is consumed by Codex itself.

It answers:

- which provider ID should Codex use
- what base URL should Codex call
- what default model name should Codex request
- whether Codex should use Responses or WebSockets

Typical fields:

- `model_provider`
- `model`
- `model_catalog_json`
- `web_search`
- `model_providers.<id>.base_url`
- `model_providers.<id>.env_key` if you want Codex → shim bearer auth
- `model_providers.<id>.wire_api`
- `model_providers.<id>.supports_websockets`

Use [examples/codex-config/config.toml](/home/vivec/codex-shim/examples/codex-config/config.toml)
as the starting point, or let `codex-shim install-codex-config` write it.

### 2. Shim config: `config.yaml`

This is consumed by `codex-shim`.

It answers:

- where the shim should listen
- which upstream provider API it should call
- how it should authenticate upstream requests
- which provider behavior preset it should use
- which models it should advertise through `/models`

Typical top-level blocks:

- `server`
- `upstream`
- `provider`
- `models`
- `reasoning`
- `state`
- `logging`

Use one of the files in `examples/<profile>/config.yaml` as the starting point.

### 3. Shim model catalog: `models.catalog`

This lives inside the shim YAML, but it deserves to be thought of separately.

It is the source of truth for the shim-native `/models` endpoint.
Current Codex also needs a startup snapshot of that catalog through top-level
`model_catalog_json` if you want correct custom-model metadata at startup and
working `/model` picker entries.

It answers:

- what model slugs Codex should see
- what context window each model should advertise
- whether each model should appear to support reasoning, search, images, patch
  editing, and so on

This is not fetched from the upstream provider automatically.

## End-To-End Setup Flow

### Step 1. Pick a provider profile

Choose the shim profile that matches the upstream API shape you want to use.

Examples:

- `deepseek-chat`
- `gemini-chat`
- `moonshot-chat`
- `openrouter-chat`
- `openrouter-responses`
- `fireworks-responses`
- `vllm-responses`
- `ollama-chat`
- `ollama-responses`
- `generic-chat`

The profile controls the shim's runtime behavior:

- chat shim vs native Responses vs stateless Responses
- reasoning extraction policy
- tool behavior defaults
- whether `previous_response_id` is local-only or upstream-native

This is configured under:

```yaml
provider:
  kind: deepseek-chat
  profile_config:
    profile: deepseek-chat
```

## Step 2. Configure the upstream provider

Point the shim at the real upstream API:

```yaml
upstream:
  base_url: "https://api.deepseek.com"
  chat_path: "/chat/completions"
  responses_path: "/responses"
  api_key_env: "DEEPSEEK_API_KEY"
```

This layer is only about shim → upstream.

It has nothing to do with Codex's own `config.toml`.

## Step 3. Describe the model catalog

Add at least one model entry:

```yaml
models:
  default: "deepseek-v4-pro"
  catalog:
    - slug: "deepseek-v4-pro"
      context_window: 131072
      reasoning_levels:
        - high
```

`models.catalog` is required. The shim validates this at startup.

### What you usually must write manually

- `slug`
- `context_window`

### What you usually should write manually

- `display_name`
- `description`
- `priority`
- `base_instructions`
- `auto_compact_token_limit`

### What can default from the provider profile

- `tool_calling`
- `vision`
- `reasoning_levels`

If omitted:

- `tool_calling` defaults from the provider capability preset
- `vision` defaults from the provider capability preset
- `reasoning_levels` defaults to `["high"]` when the profile supports
  reasoning effort, otherwise `[]`

### Model name alignment

Keep these three values aligned:

1. Codex `config.toml`: top-level `model`
2. shim YAML: `models.default`
3. shim YAML: at least one `models.catalog[*].slug`

If they drift apart, Codex may ask for a model name that the shim does not
advertise through `/models`.

## Step 4. Configure reasoning

The `reasoning` block controls shim runtime defaults:

```yaml
reasoning:
  enabled: false
  effort: high
```

This is separate from catalog metadata.

### Important distinction

- `reasoning.enabled` controls runtime/default shim behavior
- `models.catalog[*].reasoning_levels` tells Codex what the model supports

So these are valid and mean different things:

```yaml
reasoning:
  enabled: false

models:
  catalog:
    - slug: "deepseek-v4-pro"
      context_window: 131072
      reasoning_levels: [high]
```

This means:

- the model is advertised to Codex as reasoning-capable
- but the shim's runtime default starts from reasoning disabled

If you want Codex to treat the model as non-reasoning, set:

```yaml
reasoning_levels: []
```

explicitly in the catalog entry.

## Step 5. Configure Codex

The preferred path is:

```bash
codex-shim install-codex-config --config ~/.codex-shim/config.yaml
```

If you need to inspect the shape it writes, it looks like:

```toml
model_provider = "codex_shim"
model = "deepseek-v4-pro"
model_catalog_json = "/absolute/path/to/$CODEX_HOME/codex-shim/model-catalog.json"
web_search = "disabled"

[model_providers.codex_shim]
name = "codex-shim"
base_url = "http://127.0.0.1:8787/v1"
wire_api = "responses"
supports_websockets = false
```

This layer is only about Codex → shim.

Add `env_key = "LOCAL_SHIM_TOKEN"` only when you want bearer auth on that hop.

It does not replace the shim YAML.

## Step 6. Export credentials

For a local loopback shim, you typically only need one environment variable:

1. shim → upstream API key

Example:

```bash
export DEEPSEEK_API_KEY="sk-..."
```

If you do enable Codex → shim bearer auth, then add:

```bash
export LOCAL_SHIM_TOKEN="local-shim-dev-token"
```

That token is only for Codex calling the shim. It is never forwarded upstream.

## Step 7. Start the shim

```bash
./codex-shim --config examples/deepseek-chat/config.yaml
```

Then run Codex normally:

```bash
codex
```

## What `profile_config` Actually Does

`profile_config` is the shim's runtime adapter preset.

It is not:

- a Codex config block
- the model catalog itself
- upstream model metadata

It is how the shim decides how to behave toward the upstream provider.

Example:

```yaml
provider:
  kind: openrouter-responses
  profile_config:
    profile: openrouter-responses
    capabilities:
      supports_hosted_web_search: false
    extra_body:
      enable_thinking: true
```

This controls:

- endpoint mode
- reasoning policy
- tool policy
- state policy
- fine-grained capability overrides
- extra request body fields injected upstream

`profile_config` influences the default values used when the shim builds
`/models`, but it does not replace `models.catalog`.

## Conflict And Precedence Rules

These layers are conceptually separate, but you can still configure them
in contradictory ways.

### Prefer `profile_config.profile` over `provider.kind`

`provider.kind` exists as a legacy shortcut.

If `provider.profile_config` is present, treat `profile_config.profile` as the
real source of truth for runtime behavior.

### `models.catalog` overrides profile-derived defaults

If you explicitly set fields like:

- `tool_calling`
- `vision`
- `reasoning_levels`

then those explicit values win over the provider capability defaults.

### Codex `model` must match the shim catalog

Codex only knows what the shim advertises through `/models`.

So if Codex requests:

```toml
model = "foo"
```

but the shim only advertises:

```yaml
models:
  catalog:
    - slug: "bar"
```

then your setup is inconsistent.

## Recommended Starting Pattern

For most users, the safest flow is:

1. copy a bundled `examples/<profile>/config.yaml`
2. change only:
   - `upstream.api_key_env`
   - `models.default`
   - `models.catalog[*].slug`
3. run `codex-shim install-codex-config --config ~/.codex-shim/config.yaml`
4. keep the Codex `model` equal to the shim `models.default`
5. only touch `profile_config.capabilities` if you are intentionally overriding
   a built-in preset

## Related Files

- [README.md](/home/vivec/codex-shim/README.md)
- [examples/all-options.yaml](/home/vivec/codex-shim/examples/all-options.yaml)
- [examples/codex-config/config.toml](/home/vivec/codex-shim/examples/codex-config/config.toml)
- [docs/e2e.md](/home/vivec/codex-shim/docs/e2e.md)
