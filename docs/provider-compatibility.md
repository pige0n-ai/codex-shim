# Provider Compatibility Matrix

This matrix is the release-facing source of truth for bundled `provider.kind`
presets and example configs.

Two columns need special care:

- `Streaming usage` records what the upstream API documents or exposes on the
  wire. It is about runtime behavior.
- `Stateful responses` records whether upstream `previous_response_id` style
  continuation is native, or whether codex-shim must materialize history.

Compact gating in live tests is stricter than this table: a provider may
document streaming usage and still be treated as non-blocking for compact
assertions until its totals are stable in practice.

| Profile | Provider | Example model | `/chat/completions` | `/responses` | Streaming usage | Stateful responses | Auth | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `deepseek-chat` | DeepSeek | `deepseek-v4-pro` | Yes | No official support found | `stream_options.include_usage` + final usage chunk | No | API key | [DeepSeek chat completion](https://api-docs.deepseek.com/api/create-chat-completion) |
| `sglang-chat` | SGLang | `local-model` | Yes | No official support found | OpenAI-compatible streaming; usage not re-audited for chat in this pass | No | Self-hosted | [SGLang OpenAI-compatible API](https://sgl-project-sglang-93.mintlify.app/backend/openai-compatible-api) |
| `vllm-responses` | vLLM | `your-vllm-model` | Yes | Yes | Native Responses usage object | Adapter-managed | Self-hosted | [vLLM OpenAI-compatible server](https://docs.vllm.ai/en/latest/serving/openai_compatible_server/) |
| `vllm-chat` | vLLM | `your-vllm-model` | Yes | Yes | OpenAI-compatible streaming; usage not re-audited for chat in this pass | No | Self-hosted | [vLLM OpenAI-compatible server](https://docs.vllm.ai/en/latest/serving/openai_compatible_server/) |
| `ollama-responses` | Ollama | `qwen3.5:32b` | Yes | Yes | Native Responses usage object | No; stateless only | Local/self-hosted | [Ollama OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility) |
| `ollama-chat` | Ollama | `qwen3.5:32b` | Yes | Yes | OpenAI-compatible streaming | No | Local/self-hosted | [Ollama OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility) |
| `llamacpp-responses` | llama.cpp | `local-model` | Yes | Yes | Responses route exists; usage follows upstream conversion | No; chat-backed shim | Self-hosted | [llama.cpp server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) |
| `llamacpp-chat` | llama.cpp | `local-model` | Yes | Yes | OpenAI-compatible streaming | No | Self-hosted | [llama.cpp server README](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) |
| `openrouter-responses` | OpenRouter | `moonshotai/kimi-k2.6` | Yes | Shim preset kept; official `/responses` docs were not re-surfaced in this audit | Runtime usage observed in existing live coverage | No; shim materializes history | API key | [OpenRouter overview](https://openrouter.ai/docs/api-reference/overview), [streaming](https://openrouter.ai/docs/api-reference/streaming) |
| `openrouter-chat` | OpenRouter | `moonshotai/kimi-k2.6` | Yes | No official support found in current public docs | Final chunk includes usage stats | No | API key | [OpenRouter overview](https://openrouter.ai/docs/api-reference/overview), [streaming](https://openrouter.ai/docs/api-reference/streaming) |
| `alibaba-responses` | Alibaba DashScope / Qwen | `qwen3.6-plus` | Yes | Yes | Native Responses usage object | Yes | API key | [Alibaba OpenAI compatibility](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope) |
| `alibaba-chat` | Alibaba DashScope / Qwen | `qwen3.6-plus` | Yes | Yes | `stream_options.include_usage` documented | No | API key | [Alibaba OpenAI compatibility](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope) |
| `groq-responses` | Groq | `llama-3.3-70b-versatile` | Yes | Yes | Native Responses usage object | No; stateless constraints in current docs | API key | [Groq Responses API](https://console.groq.com/docs/api-reference#responses-create), [Groq text chat](https://console.groq.com/docs/text-chat) |
| `groq-chat` | Groq | `llama-3.3-70b-versatile` | Yes | Yes | Usage behavior not re-audited in this pass | No | API key | [Groq text chat](https://console.groq.com/docs/text-chat) |
| `together-chat` | Together AI | `meta-llama/Llama-3.3-70B-Instruct-Turbo` | Yes | No official support found | Usage behavior not re-audited in this pass | No | API key | [Together chat completions](https://docs.together.ai/reference/chat-completions-1) |
| `fireworks-responses` | Fireworks AI | `accounts/fireworks/models/qwen3-235b-a22b` | Yes | Yes | Native Responses usage object | Yes | API key | [Fireworks Responses API](https://docs.fireworks.ai/api-reference/post-responses) |
| `fireworks-chat` | Fireworks AI | `accounts/fireworks/models/qwen3-235b-a22b` | Yes | Yes | Last streamed chunk includes usage | No | API key | [Fireworks chat completions](https://docs.fireworks.ai/api-reference/post-chatcompletions) |
| `xai-responses` | xAI | `grok-4.20-reasoning` | Yes | Yes | Native Responses usage object | Yes | API key | [xAI Responses API](https://docs.x.ai/docs/api-reference) |
| `xai-chat` | xAI | `grok-4.20-reasoning` | Yes | Yes | Streaming chunks can include running usage and cost | No | API key | [xAI chat completions](https://docs.x.ai/docs/api-reference#chat-completions) |
| `bedrock-responses` | Amazon Bedrock | `amazon.nova-pro-v1:0` | Yes | Yes | Native Responses usage object | Yes | API key or command-fetched bearer | [Bedrock API compatibility](https://docs.aws.amazon.com/bedrock/latest/userguide/models-api-compatibility.html), [Mantle OpenAI example](https://docs.aws.amazon.com/bedrock/latest/userguide/inference-chat-completions.html) |
| `bedrock-chat` | Amazon Bedrock | `amazon.nova-pro-v1:0` | Yes | Yes | Usage behavior not re-audited in this pass | No | API key or command-fetched bearer | [Bedrock chat completions](https://docs.aws.amazon.com/bedrock/latest/userguide/inference-chat-completions.html) |
| `gemini-chat` | Gemini API / AI Studio | `gemini-3-flash-preview` | Yes | No official support found | `stream_options.include_usage` documented | No | API key | [Gemini OpenAI compatibility](https://ai.google.dev/gemini-api/docs/openai) |
| `vertex-chat` | Vertex AI | `gemini-2.5-flash` | Yes | No official support found | Usage behavior not re-audited in this pass | No | OAuth bearer via `auth_command` | [Vertex AI OpenAI compatibility](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/migrate/openai/overview), [Vertex AI start guide](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/start/openai) |
| `minimax-chat` | MiniMax | `MiniMax-M2.7` | Yes | No official support found | `stream_options` exists; usage final-chunk semantics not re-audited in this pass | No | API key | [MiniMax OpenAI compatibility](https://platform.minimaxi.com/docs/api-reference/text-openai-api) |
| `moonshot-chat` | Moonshot / Kimi | `kimi-k2.6` | Yes | No official support found | `stream_options.include_usage` documented | No | API key | [Kimi API overview](https://platform.kimi.ai/docs/api/overview) |
| `zai-chat` | Z.AI / GLM | `glm-5.1` | Yes | No official support found | Usage behavior not re-audited in this pass | No | API key | [Z.AI OpenAI Python SDK overview](https://docs.z.ai/guides/develop/openai/python) |
| `generic-chat` | User-supplied OpenAI-compatible upstream | `model-slug` | Depends on upstream | Depends on upstream | Optimistic request path; verify against your upstream docs | No | User-defined | N/A |

## Notes

- `openrouter-responses` remains bundled because it is covered by existing live
  tests and current shim behavior, but its public official `/responses`
  documentation did not surface with the same clarity as the providers above.
- `llamacpp-responses` and `ollama-responses` are intentionally marked
  non-stateful even though they expose `/responses`: the shim must continue to
  materialize history for them.
- `generic-chat` is intentionally conservative for public docs. It is a preset
  for upstreams that are OpenAI-compatible enough to work with the chat shim,
  not a promise that every arbitrary provider implements the full feature set.
