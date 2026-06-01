# CLI Reference

## `codex-shim` (server mode)

Start the shim server. Uses `~/.codex-shim/config.yaml` by default.

```bash
codex-shim
codex-shim --config /path/to/config.yaml
codex-shim --listen 0.0.0.0:8787
```

If no subcommand is given, no `--config` path is provided, and the default
`~/.codex-shim/config.yaml` does not exist, server mode launches the interactive
`setup --yolo` first-run flow instead of failing immediately.

## `setup`

Interactive setup wizard with step-by-step output. Walks through provider
category, provider profile, API key, listen address, model config, and optional
upstream connectivity probing.

```bash
codex-shim setup                    # interactive, write config, print next steps
codex-shim setup --integrate        # setup + validate + install Codex files
codex-shim setup --yolo             # setup + validate + install + start server
codex-shim setup --non-interactive  # silent defaults → deepseek-chat
codex-shim setup --output ./my-config.yaml
```

The wizard asks for:
1. Provider category (Hosted API / Local / Generic)
2. Provider profile (27 built-in presets)
3. Upstream base URL (local providers only)
4. API key environment variable name
5. **Listen address** — host:port the shim server listens on. Written to
   `config.yaml` as `server.listen` and used as `base_url` in Codex `config.toml`.
   Default: `127.0.0.1:8787`.
6. Model slug, context window (supports k/K/m/M suffixes), reasoning settings,
   and Codex catalog metadata. Generated catalog entries enable
   `apply_patch_tool_type: freeform`.
7. Optional upstream connectivity probe

Generated configs include the current streaming/debug defaults:
`stream_max_retries: 2`, `downstream_heartbeat_seconds: 30`,
`debug_artifact_ttl_seconds: 600`, and
`failed_debug_artifact_ttl_seconds: 0`.

## `integrate`

Validate config and install Codex startup catalog + update `config.toml`.
Reads `server.listen` from config to construct the Codex `base_url` — no
hardcoded address.

```bash
codex-shim integrate                       # validate + install Codex files
codex-shim integrate --start               # also start the server
codex-shim integrate --dry-run             # preview without writing
codex-shim integrate --config /path/to/config.yaml
codex-shim integrate --project-dir /path/to/repo --trust-project  # desktop
codex-shim integrate --env-key LOCAL_SHIM_TOKEN                    # bearer auth
```

## `validate`

Offline YAML validation. Non-zero exit on errors.

```bash
codex-shim validate
codex-shim validate --config /path/to/config.yaml
codex-shim validate --check-upstream
```

## `config-show`

Inspect the resolved configuration.

```bash
codex-shim config-show          # summary (default)
codex-shim config-show yaml     # full YAML
codex-shim config-show json     # full JSON
```

## `generate-catalog`

Generate a model catalog JSON.

```bash
codex-shim generate-catalog --config ~/.codex-shim/config.yaml
codex-shim generate-catalog --config ... --output /tmp/catalog.json
```

Generated catalog entries advertise `apply_patch_tool_type: freeform` by
default so Codex can expose its patch editing tool for shell-capable models.

## `explain-catalog`

Explain what a model catalog JSON means to Codex.

```bash
codex-shim explain-catalog /path/to/catalog.json
```

## `probe`

Probe an upstream endpoint and report detected capabilities.

```bash
codex-shim probe --upstream-base https://api.deepseek.com --upstream-key-env DEEPSEEK_API_KEY
codex-shim probe --provider deepseek-chat
```

## `doctor desktop`

Validate desktop-oriented Codex project wiring.

```bash
codex-shim doctor desktop \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo
```

Checks: project `.codex/config.toml`, trust, `model_provider`, `model_catalog_json`,
`wire_api`, `supports_websockets`, `web_search` compatibility.
