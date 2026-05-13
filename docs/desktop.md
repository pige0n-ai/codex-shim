# Codex Desktop App Support

Supported operating contract for using `codex-shim` with the Codex desktop app.

## Supported

- macOS-first support target
- One project → one shim upstream
- Stable provider identity: `model_provider = "codex_shim"`
- Project-scoped config: `<repo>/.codex/config.toml`
- Project-scoped catalog: `<repo>/.codex/codex-shim/model-catalog.json`
- Trusted-project flow (`--trust-project`)
- Shim-managed history/resume within the trusted project
- `shell`, `apply_patch`, and standard function-tool flows

Install and validate:

```bash
codex-shim integrate \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo \
  --trust-project

codex-shim doctor desktop \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo
```

`doctor desktop` checks: project config, trust entry, stable `model_provider`
and `model_catalog_json`, `wire_api = "responses"`, `supports_websockets = false`,
and `web_search` compatibility.

## Gated

- Old non-shim desktop threads resuming with their original provider context.
  This depends on Codex desktop's thread restoration logic, not the shim.
  See [openai/codex#15219](https://github.com/openai/codex/issues/15219),
  [openai/codex#15494](https://github.com/openai/codex/issues/15494).

## Unsupported

- Multiple upstreams behind one provider identity
- Desktop automations as a shim compatibility target
- Hosted tools on chat-shim paths
- `computer_use`, `mcp`, `code_interpreter` through unsupported upstreams

## Web Search Rules

- Chat shim profiles → `web_search = "disabled"`.
- Native/stateless Responses → `cached` or `live` only when the profile
  advertises hosted search and every catalog model has `supports_search_tool = true`.
- If the contract is not met, installation fails or `doctor desktop` reports it.
