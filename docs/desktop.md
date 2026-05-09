# Codex Desktop App Support

This document defines the supported operating contract for using
`codex-shim` with the Codex desktop app.

The goal is a **deliverable desktop setup**, not a vague claim that "CLI
compatibility probably also works in the app."

## Supported

- macOS-first support target
- one project bound to one shim upstream at a time
- stable desktop provider identity: `model_provider = "codex_shim"`
- project-scoped config at `<repo>/.codex/config.toml`
- project-scoped startup catalog at
  `<repo>/.codex/codex-shim/model-catalog.json`
- trusted-project flow via `install-codex-config --project-dir ... --trust-project`
- shim-managed thread history and resume inside that trusted project
- `shell`, `apply_patch`, and normal function-tool flows

Install a project-scoped desktop config with:

```bash
codex-shim install-codex-config \
  --config ~/.codex-shim/config.yaml \
  --project-dir /absolute/path/to/repo \
  --trust-project
```

Validate it with:

```bash
codex-shim doctor desktop \
  --config ~/.codex-shim/config.yaml \
  --project-dir /absolute/path/to/repo
```

`doctor desktop` checks:

- project `.codex/config.toml`
- project trust in the global Codex config
- stable `model_provider`
- stable `model_catalog_json`
- `wire_api = "responses"`
- `supports_websockets = false`
- `web_search` compatibility with the active shim profile and catalog

## Gated

The following is **not** promised by `codex-shim` alone:

- old non-shim desktop threads resuming with their original provider context

That behavior depends on Codex desktop's own thread restoration logic. Treat it
as an external gate, not an adapter guarantee.

Relevant public reports:

- [openai/codex#15219](https://github.com/openai/codex/issues/15219)
- [openai/codex#15494](https://github.com/openai/codex/issues/15494)

## Unsupported

- multiple upstreams hidden behind one desktop provider identity at the same time
- desktop automations as a shim compatibility target
- fake compatibility for raw hosted tools on chat-shim paths
- raw `computer_use`, raw `mcp`, or raw `code_interpreter` support through
  unsupported upstreams

## Web Search Rules

- Chat Completions shim profiles force `web_search = "disabled"`.
- Native/stateless Responses profiles may use `cached` or `live` only when:
  - the shim provider profile advertises hosted web search support
  - every advertised catalog model sets `supports_search_tool = true`

If that contract is not met, installation fails or `doctor desktop` reports the
configuration as unsupported. The adapter does not silently keep a broken search
configuration in place.
