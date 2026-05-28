# v0.6.0 Notes

This release contains breaking Chat Completions adapter changes.

## Streaming Diagnostics

Failed streamed `/chat/completions` relays now persist richer upstream body
failure diagnostics in debug artifacts. When the upstream HTTP request has
already returned success but the response body later fails, `upstream_error`
records the response status, HTTP version, redacted response headers, reqwest
error classification, source chain, and a bounded tail of raw upstream SSE data.
This makes provider transport/body failures distinguishable from mapper errors
after the benchmark run has finished.

## Multimodal Tool Outputs

`function_call_output` parts may now include `input_image`. The shim maps the
tool output itself to a textual `role: tool` acknowledgement and appends
synthetic `role: user` multimodal messages for the images. This avoids relying
on provider-specific support for multimodal `role: tool` messages.

## Structured `apply_patch`

Structured upstream `apply_patch` now requires a non-empty `raw_patch` field.
The AST is still preferred when it compiles successfully; if AST compilation
fails, the shim passes `raw_patch` through to Codex apply_patch unchanged.

`models.catalog[*].apply_patch_upstream_strict` is a new opt-in boolean for
setting `strict: true` on the upstream structured apply_patch function. It
defaults to `false` because OpenAI-compatible providers vary in how much JSON
Schema they accept in strict mode.
