# codex-shim

[English](README.md)

`codex-shim` 是一个本地适配器，用来把
[Codex](https://developers.openai.com/codex) 的自定义模型提供商接到非
OpenAI 或自托管模型上。Codex 只需要面对一个熟悉的 `/v1/responses`
接口；shim 会把请求翻译到上游 Chat Completions，或透传到原生 Responses
接口。

适用场景包括：你想让 Codex CLI 或 Codex Desktop 使用 DeepSeek、
OpenRouter、xAI、Gemini、Groq、Ollama、vLLM、llama.cpp、SGLang，或其他
OpenAI 兼容后端，同时不想手工维护复杂的 Codex 配置。

## 架构

```text
Codex (wire_api = "responses") -> codex-shim /v1/responses
                                      |-> NativeResponses     -> 上游 /v1/responses
                                      |-> StatelessResponses  -> 上游 /v1/responses
                                      |                         由 shim 补全历史
                                      `-> ChatCompletionsShim -> 上游 /v1/chat/completions
```

Codex 侧始终使用 Responses API。Chat Completions 只是 shim 内部与上游通信时
可能使用的协议。

## 平台支持

发布包提供 Linux `x86_64`（musl static）、macOS `x86_64`/`aarch64`、Windows
`x86_64` 的二进制。

Codex CLI 在三个平台上都可用：启动 shim，运行 `integrate`，然后正常启动
`codex`。

Codex Desktop 目前以 macOS 为主要支持目标。macOS 版本会读取
`model_catalog_json` 并显示模型列表。Windows 上模型选择器可能只显示
“Custom”，即使 catalog 里有多个模型；实际请求仍会使用 `config.toml` 顶层
`model` 指定的模型。

如果 Codex Desktop 的 agent 环境设置为 WSL，请注意请求会从 WSL 网络命名空间
发出。只绑定在 Windows 主机 `127.0.0.1` 的 shim 通常不可达。此时应把 shim
绑定到 `0.0.0.0` 或 WSL 可访问的主机 IP，并在 `config.toml` 中使用该地址。
推荐用 Windows 二进制安装 Windows 侧 Codex 配置，再在 WSL 内用 Linux 二进制
和同一份 shim 配置启动监听。

## 快速开始

从 release 下载对应平台的原始二进制即可。`refs.zip` 只包含参考文件：
`examples/`、`README.md` 和 `LICENSE`。

首次配置：

```bash
./codex-shim setup --yolo
```

向导会询问 provider、模型、API key 环境变量名和监听地址，然后写入 shim 配置、
安装 Codex provider 文件，并启动服务。如果直接运行 `codex-shim`，没有子命令且
默认配置不存在，也会自动进入这个首次配置流程。

分步执行：

```bash
./codex-shim setup              # 交互式写入配置
./codex-shim setup --integrate  # 配置并安装 Codex 文件
export DEEPSEEK_API_KEY="sk-..."
./codex-shim integrate --start --config ~/.codex-shim/config.yaml
```

从源码构建：

```bash
cargo build --release -p codex-shim
./target/release/codex-shim --config examples/deepseek-chat/config.yaml
```

## 核心概念

需要保持三处配置一致：

| 层级 | 文件 | 用途 |
| --- | --- | --- |
| Codex 配置 | `$CODEX_HOME/config.toml` | 告诉 Codex 如何访问 shim |
| Shim 配置 | `~/.codex-shim/config.yaml` | 告诉 shim 如何访问上游 provider |
| 模型目录 | shim YAML 中的 `models.catalog` | 告诉 Codex 可用模型、能力和工具 |

这三个值应对齐：Codex 的 `model`、shim 的 `models.default`，以及至少一个
`models.catalog[*].slug`。

## CLI

```bash
codex-shim [OPTIONS] [COMMAND]

Commands:
  setup                交互式配置向导，推荐使用
  integrate            校验配置并安装 Codex 启动 catalog
  validate             检查 YAML 配置
  config-show          查看解析后的配置 summary/yaml/json
  generate-catalog     生成模型 catalog JSON
  explain-catalog      解释 Codex 如何理解模型 catalog
  probe                探测上游 endpoint 能力
  doctor               检查 Codex Desktop 项目配置

Options:
  -c, --config <PATH>       配置文件路径，默认 ~/.codex-shim/config.yaml
      --listen <ADDR>       监听地址，默认 127.0.0.1:8787
```

完整命令说明见 [docs/cli.md](docs/cli.md)。

## 配置

最小配置示例：

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
      apply_patch_tool_type: freeform
```

完整配置参考见 [docs/configuration.md](docs/configuration.md)。所有支持的字段和注释
见 [examples/all-options.yaml](examples/all-options.yaml)。

## 认证

有两层彼此独立的认证：

1. Codex -> shim：本地跳转的可选 bearer auth，通过 `accepted_bearer_tokens`
   配置。
2. Shim -> 上游：真实 provider 凭据，通常通过 `upstream.api_key_env` 指向环境变量，
   例如 `DEEPSEEK_API_KEY`。

shim 不会把本地 bearer token 转发给上游 provider。

## Provider Profiles

内置 27 个 profile，覆盖托管 API（DeepSeek、OpenRouter、xAI、Groq、Gemini 等）、
本地/自托管服务（Ollama、vLLM、llama.cpp、SGLang），以及通用 OpenAI 兼容上游。

兼容性矩阵和 provider 说明见
[docs/provider-compatibility.md](docs/provider-compatibility.md)。

## 运行时行为

- Chat Completions 流式请求只会在下游 SSE 尚未开始前由 shim 重试。一旦 Codex 已经
  收到事件，中途断流会变成 `response.failed` 和 debug artifact，让 Codex 使用自己的
  turn-level retry。
- `upstream.downstream_heartbeat_seconds` 会在长时间 reasoning-only、usage-only 或
  custom tool 参数累积期间发送轻量 `response.in_progress`，避免 Codex 误判 SSE idle
  timeout。设为 `0` 可关闭。
- 失败的 raw request/SSE debug artifact 默认不自动过期：
  `state.failed_debug_artifact_ttl_seconds: 0`。成功 artifact 仍使用
  `state.debug_artifact_ttl_seconds`。
- 显式 catalog 条目如果希望 Codex 暴露 patch 编辑，应设置
  `apply_patch_tool_type: freeform`。Chat 上游可额外选择
  `apply_patch_upstream_tool_type: structured`，但结构化调用必须包含 `raw_patch`。
- 对 Chat 上游，多模态 tool output 会被映射为文本 `role: tool` 确认消息，再追加合成
  user 图片消息，以提高 provider 兼容性。

## Codex Desktop

项目级安装：

```bash
codex-shim integrate \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo \
  --trust-project

codex-shim doctor desktop \
  --config ~/.codex-shim/config.yaml \
  --project-dir /path/to/repo
```

Desktop 支持策略是保守的：一个可信项目、一个稳定
`model_provider = "codex_shim"`、一个项目内模型 catalog。详情见
[docs/desktop.md](docs/desktop.md)。

## 安全提示

首次使用前建议备份 `$CODEX_HOME/config.toml`。codex-shim 更新该文件时会保留最多
四份滚动备份（`.bak.0` 到 `.bak.3`），但自己留一份仍然更安心。

Codex 的 thread history 绑定在 `model_provider` key 上。修改 `model_provider` 可能会
让旧线程在 UI 中暂时不可见；它们没有被删除，恢复旧配置或切回原 provider key 即可。

Chat Completions 兼容性取决于上游 provider 的工具和流式行为。如果某个 provider
接近可用但还差一点，请
[file an issue](https://github.com/pige0n-ai/codex-shim/issues)。

## 测试

```bash
cargo test                                               # 单元测试 + 集成测试
cargo test -p e2e-codex --test codex_mock                # mock E2E，离线
cargo test -p e2e-codex --test codex_mock -- --ignored   # 加上 Codex blackbox

CODEX_SHIM_E2E_KEYS=secrets/e2e.providers.toml \
cargo test -p e2e-codex --test codex_live -- --ignored --nocapture
```

更多说明见 [docs/e2e.md](docs/e2e.md)。

## License

MIT
