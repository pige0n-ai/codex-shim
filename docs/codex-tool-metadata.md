# Extracting Codex Tool Metadata

This note is for maintainers who need to reconstruct which tools Codex can send
to a model and which upstream switches control them.

This repository does not vendor `openai/codex`. Clone or fetch the upstream
repository first:

```bash
git clone https://github.com/openai/codex.git /tmp/openai-codex
cd /tmp/openai-codex
```

Use the checked-out `codex-rs` tree as the source of truth.

## Files to Inspect

| Question | Upstream path in `openai/codex` | What to extract |
| --- | --- | --- |
| What metadata can a model advertise? | `codex-rs/protocol/src/openai_models.rs` | `ModelInfo`, especially `shell_type`, `apply_patch_tool_type`, `supports_search_tool`, `web_search_tool_type`, `supports_image_detail_original`, `experimental_supported_tools`, and input modalities. |
| How does model metadata become runtime tool switches? | `codex-rs/tools/src/tool_config.rs` | `ToolsConfig::new`, which copies model metadata and feature gates into a concrete tool configuration. |
| Which tool executors are registered? | `codex-rs/core/src/tools/spec_plan.rs` | `collect_tool_executors`, `build_model_visible_specs_and_registry`, `hosted_model_tool_specs`, `append_tool_search_executor`, and `prepend_code_mode_executors`. |
| Where are model-visible schemas defined? | `codex-rs/core/src/tools/handlers/*_spec.rs` and `codex-rs/tools/src/tool_spec.rs` | Function, freeform, hosted, and namespace tool schemas. |
| How are app/plugin/MCP tools added? | `codex-rs/core-plugins`, `codex-rs/mcp-server`, and extension-tool handling in `spec_plan.rs` | Deferred tools, discoverable tools, MCP resource helpers, and extension tool adapters. |

Useful search commands:

```bash
rg -n "struct ModelInfo|apply_patch_tool_type|shell_type|supports_search_tool" codex-rs
rg -n "struct ToolsConfig|fn new\\(|collect_tool_executors|hosted_model_tool_specs" codex-rs
rg -n "impl .*Handler|ToolExposure|ToolSpec::|ResponsesApiNamespace" codex-rs/core/src/tools codex-rs/tools/src
```

## Tool Exposure Checklist

| Tool family | Upstream switch or source | Model-visible when |
| --- | --- | --- |
| `exec_command` | `ModelInfo.shell_type`, shell feature flags, unified-exec feature | Environment exists, shell tools are enabled, and selected shell type is `unified_exec`. |
| `write_stdin` | Same as `exec_command` | Same as `exec_command`. |
| Classic shell command | `ModelInfo.shell_type`, shell feature flags | Environment exists and selected shell type is classic/default/local shell. |
| `apply_patch` | `ModelInfo.apply_patch_tool_type` | Environment exists, shell is not disabled, and `apply_patch_tool_type` is present. Current callable type is `freeform`. |
| `view_image` | Environment mode, `supports_image_detail_original` | Environment exists; original-detail support is separately controlled by model metadata. |
| Plan tool | Built-in executor in `spec_plan.rs` | Registered by core, then filtered by exposure/code-mode rules. |
| MCP resource tools | MCP tool availability | MCP tools are configured. |
| MCP server tools | MCP descriptors | MCP tools are configured; deferred tools may be discovered through `tool_search`. |
| `tool_search` | `supports_search_tool`, tool-search feature, namespace tools | Search is supported and at least one deferred tool exists. |
| Plugin install request | Tool suggestion/apps/plugins features plus discoverable tools | Discoverable plugin tools are available. |
| Hosted web search | Web-search mode/config and `web_search_tool_type` | Hosted web search produces a tool spec. |
| Hosted image generation | Image-generation feature, auth entitlement, model support | Image generation is enabled and allowed for the current auth/model. |
| Permission tools | Permission feature flags | Request-permission or exec-approval features are enabled. |
| Multi-agent tools | Collaboration or multi-agent feature flags | Multi-agent/collaboration features are enabled. |
| Goal tools | Goals feature flag | Goals are enabled. |
| Code-mode tools | Code-mode feature flags | Code mode is enabled; nested tools may be wrapped behind code-mode execute/wait. |
| Agent-job tools | Agent job feature flags and worker source | Agent jobs are enabled; worker reporting is source-gated. |
| Dynamic tools | Runtime `DynamicToolSpec` list | Dynamic tools are present and convertible. |
| Extension tools | Extension executors | Extension tools are installed and do not collide with reserved/core names. |
| Test sync tool | `experimental_supported_tools` contains `test_sync_tool` | Explicitly advertised by model metadata. |

## `codex-shim` Mapping

For `codex-shim`, the most important catalog fields are:

```yaml
models:
  catalog:
    - slug: example-model
      tool_calling: true
      supports_search_tool: false
      supports_image_detail_original: false
      apply_patch_tool_type: freeform
      apply_patch_upstream_tool_type: structured
      apply_patch_upstream_strict: false
```

`tool_calling: true` makes `codex-shim` advertise a Codex shell-capable model.
`apply_patch_tool_type: freeform` is required if Codex should send a callable
`apply_patch` tool. Base-instruction text about patching is not enough.
For Chat Completions upstreams, `apply_patch_upstream_tool_type: structured`
keeps the Codex-facing freeform capability but exposes a structured JSON AST
tool upstream; the shim compiles that AST back into Codex apply_patch syntax.
Structured apply_patch now requires a non-empty `raw_patch` field in every
upstream tool call. If the AST cannot be compiled but `raw_patch` is present,
the shim passes the raw Codex patch through unchanged. Set
`apply_patch_upstream_strict: true` only for providers that accept this JSON
schema in strict function mode.

When `models.catalog` is omitted and `models.default` is set, `codex-shim`
auto-generates a single catalog entry. That generated entry should advertise
`apply_patch_tool_type: freeform`. Explicit catalog entries remain authoritative:
if a config sets `apply_patch_tool_type: null`, `codex-shim` leaves `apply_patch`
disabled for that model rather than silently overriding the config.
