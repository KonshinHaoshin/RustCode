# RustCode

`RustCode` 是基于 [lorryjovens-hub/claude-code-rust](https://github.com/lorryjovens-hub/claude-code-rust) fork 后继续改造的版本。这个分支不再把自己描述为“原版项目的完整替代”，而是聚焦在几个更实用的增强点：

- 主命令统一为 `rustcode`
- 支持多 provider 选择
- 支持自定义 provider
- 自定义 provider 可选 `openai` 或 `anthropic` 协议
- 支持全局 fallback 链，当前模型失败时自动切换到下一个目标
- 默认 UI 主题改为蓝色系

## 当前能力

### Provider / Protocol

内置 provider 预设：

- `deepseek`
- `openai`
- `dashscope`
- `openrouter`
- `ollama`
- `custom`

其中：

- 大多数预设默认走 OpenAI 兼容格式
- `custom` provider 可以显式选择：
  - `openai`
  - `anthropic`

### Fallback

当当前请求失败时，`RustCode` 会按配置的 fallback 链尝试下一个目标。当前实现的触发条件主要包括：

- 网络错误
- 超时
- 401 / 403 / 404
- 409 / 429
- 5xx

## 安装

### 本地构建

```bash
cargo build --release
```

主二进制输出：

- Linux/macOS: `target/release/rustcode`
- Windows: `target/release/rustcode.exe`

### 本地安装

```bash
cargo install --path .
```

安装完成后可直接运行：

```bash
rustcode --help
rustcode --version
```

## 快速开始

首次直接运行 `rustcode` 会自动进入全屏 TUI，并在没有完成配置时自动弹出 onboarding。

```bash
rustcode
```

如果你想手动再次进入交互式配置主模型和 fallback 链，也可以直接运行：

```bash
rustcode config onboard
```

显式进入全屏 TUI：

```bash
rustcode tui
```

### 1. 选择 provider

```bash
rustcode config set provider deepseek
rustcode config set api_key "your-api-key"
```

或：

```bash
rustcode config set provider openai
rustcode config set api_key "your-openai-key"
rustcode config set model gpt-4.1-mini
```

### 2. 配置自定义 provider

OpenAI 兼容：

```bash
rustcode config set provider custom
rustcode config set protocol openai
rustcode config set custom_provider_name my-gateway
rustcode config set base_url https://api.example.com
rustcode config set api_key "your-api-key"
rustcode config set model custom-model
```

Anthropic 兼容：

```bash
rustcode config set provider custom
rustcode config set protocol anthropic
rustcode config set custom_provider_name my-anthropic-gateway
rustcode config set base_url https://api.example.com
rustcode config set api_key "your-api-key"
rustcode config set model claude-3-5-sonnet-20241022
```

### 3. 配置 fallback

最简单的链式配置：

```bash
rustcode config set fallback.enabled true
rustcode config set fallback.chain "deepseek:deepseek-chat,openai:gpt-4.1-mini"
```

如果需要更复杂的 custom target，直接写 JSON：

```bash
rustcode config set fallback.chain "[{\"provider\":\"openai\",\"model\":\"gpt-4.1-mini\"},{\"provider\":\"custom\",\"protocol\":\"anthropic\",\"custom_provider_name\":\"backup-gateway\",\"base_url\":\"https://api.example.com\",\"model\":\"claude-3-5-sonnet-20241022\"}]"
```

## 使用示例

```bash
rustcode query --prompt "分析这个仓库的结构"
rustcode
rustcode repl
rustcode config show
```

其中：

- `rustcode` 默认进入新的全屏 TUI
- `rustcode repl` 保留为 legacy 行式 REPL

## 配置文件位置

当前配置目录：

- `~/.rustcode/`

如果发现旧版本的 `~/.claude-code/settings.json`，程序会迁移到新目录。

## 说明

这个 fork 的目标是把项目逐步整理成一个更明确的多 provider Rust CLI：

- 命令名统一
- 配置模型统一
- 协议层清晰
- 文档与安装路径一致

上游项目和仓库中的 TS 参考树仍然保留，用于对照和迁移参考。
