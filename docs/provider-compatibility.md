# Provider Compatibility Matrix

Release-facing source of truth for bundled provider profiles. Each preset
controls the shim's runtime behavior: endpoint mode, reasoning policy, tool
policy, and state handling.

| Profile | Provider | Example model | Chat | Responses | Streaming usage | Stateful | Auth |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `deepseek-chat` | DeepSeek | `deepseek-v4-pro` | ✓ | — | `stream_options.include_usage` + final usage chunk | No | API key |
| `minimax-chat` | MiniMax | `MiniMax-M2.7` | ✓ | — | `stream_options` documented | No | API key |
| `moonshot-chat` | Moonshot / Kimi | `kimi-k2.6` | ✓ | — | `stream_options.include_usage` documented | No | API key |
| `zai-chat` | Z.AI / GLM | `glm-5.1` | ✓ | — | Not re-audited | No | API key |
| `gemini-chat` | Gemini / AI Studio | `gemini-3-flash-preview` | ✓ | — | `stream_options.include_usage` documented | No | API key |
| `vertex-chat` | Vertex AI | `gemini-2.5-flash` | ✓ | — | Not re-audited | No | OAuth via `auth_command` |
| `alibaba-chat` | Alibaba / Qwen | `qwen3.6-plus` | ✓ | ✓ | `stream_options.include_usage` documented | No | API key |
| `alibaba-responses` | Alibaba / Qwen | `qwen3.6-plus` | ✓ | ✓ | Native Responses usage object | Yes | API key |
| `fireworks-chat` | Fireworks AI | `accounts/fireworks/models/qwen3-235b-a22b` | ✓ | ✓ | Last chunk includes usage | No | API key |
| `fireworks-responses` | Fireworks AI | `accounts/fireworks/models/qwen3-235b-a22b` | ✓ | ✓ | Native Responses usage object | Yes | API key |
| `xai-chat` | xAI | `grok-4.20-reasoning` | ✓ | ✓ | Running usage + cost in stream | No | API key |
| `xai-responses` | xAI | `grok-4.20-reasoning` | ✓ | ✓ | Native Responses usage object | Yes | API key |
| `bedrock-chat` | Amazon Bedrock | `amazon.nova-pro-v1:0` | ✓ | ✓ | Not re-audited | No | API key or `auth_command` |
| `bedrock-responses` | Amazon Bedrock | `amazon.nova-pro-v1:0` | ✓ | ✓ | Native Responses usage object | Yes | API key or `auth_command` |
| `openrouter-chat` | OpenRouter | `moonshotai/kimi-k2.6` | ✓ | — | Final chunk includes usage | No | API key |
| `openrouter-responses` | OpenRouter | `moonshotai/kimi-k2.6` | ✓ | ✓ | Runtime usage observed | No | API key |
| `groq-chat` | Groq | `llama-3.3-70b-versatile` | ✓ | ✓ | Not re-audited | No | API key |
| `groq-responses` | Groq | `llama-3.3-70b-versatile` | ✓ | ✓ | Native Responses usage object | No (stateless) | API key |
| `together-chat` | Together AI | `meta-llama/Llama-3.3-70B-Instruct-Turbo` | ✓ | — | Not re-audited | No | API key |
| `ollama-chat` | Ollama | `qwen3.5:32b` | ✓ | ✓ | OpenAI-compatible streaming | No | Local |
| `ollama-responses` | Ollama | `qwen3.5:32b` | ✓ | ✓ | Native Responses usage object | No (stateless) | Local |
| `llamacpp-chat` | llama.cpp | `local-model` | ✓ | ✓ | OpenAI-compatible streaming | No | Self-hosted |
| `llamacpp-responses` | llama.cpp | `local-model` | ✓ | ✓ | Follows upstream conversion | No (chat-backed) | Self-hosted |
| `vllm-chat` | vLLM | `your-vllm-model` | ✓ | ✓ | OpenAI-compatible streaming | No | Self-hosted |
| `vllm-responses` | vLLM | `your-vllm-model` | ✓ | ✓ | Native Responses usage object | Adapter-managed | Self-hosted |
| `sglang-chat` | SGLang | `local-model` | ✓ | — | OpenAI-compatible streaming | No | Self-hosted |
| `generic-chat` | User-supplied | `model-slug` | Depends | Depends | Verify against upstream docs | No | User-defined |

Evidence links: [DeepSeek](https://api-docs.deepseek.com/api/create-chat-completion),
[OpenRouter](https://openrouter.ai/docs/api-reference/overview),
[Groq](https://console.groq.com/docs/api-reference#responses-create),
[Fireworks](https://docs.fireworks.ai/api-reference/post-responses),
[xAI](https://docs.x.ai/docs/api-reference),
[Bedrock](https://docs.aws.amazon.com/bedrock/latest/userguide/models-api-compatibility.html),
[Alibaba](https://help.aliyun.com/zh/model-studio/compatibility-of-openai-with-dashscope),
[Gemini](https://ai.google.dev/gemini-api/docs/openai),
[Vertex AI](https://docs.cloud.google.com/vertex-ai/generative-ai/docs/migrate/openai/overview),
[Together](https://docs.together.ai/reference/chat-completions-1),
[vLLM](https://docs.vllm.ai/en/latest/serving/openai_compatible_server/),
[Ollama](https://docs.ollama.com/api/openai-compatibility),
[llama.cpp](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md),
[SGLang](https://sgl-project-sglang-93.mintlify.app/backend/openai-compatible-api),
[MiniMax](https://platform.minimaxi.com/docs/api-reference/text-openai-api),
[Kimi](https://platform.kimi.ai/docs/api/overview),
[Z.AI](https://docs.z.ai/guides/develop/openai/python).

## Notes

- `openrouter-responses` remains bundled (covered by live tests), but official
  `/responses` docs weren't re-surfaced with full clarity.
- `ollama-responses` and `llamacpp-responses` expose `/responses` but are
  non-stateful — the shim materializes history for them.
- `generic-chat` is for upstreams that are OpenAI-compatible enough to work with
  the chat shim. It's not a universal compatibility promise.
