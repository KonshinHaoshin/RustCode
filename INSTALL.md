# RustCode 安装指南

## 方式一：直接本地安装

```bash
cargo install --path .
```

安装完成后：

```bash
rustcode --help
```

## 方式二：先构建再手动拷贝

```bash
cargo build --release
```

产物位置：

- Linux/macOS: `target/release/rustcode`
- Windows: `target/release/rustcode.exe`

## 方式三：使用仓库脚本

Linux/macOS：

```bash
./scripts/install-linux.sh
```

Windows：

```powershell
.\scripts\install-windows.ps1
```

## 初始配置

推荐直接启动：

```bash
rustcode
```

首次启动会自动进入全屏 TUI，并在未完成配置时自动弹出 onboarding。

如果你想随时重新进入交互式引导：

```bash
rustcode config onboard
```

手动配置示例：

```bash
rustcode config set provider deepseek
rustcode config set api_key "your-api-key"
rustcode config set model deepseek-chat
```

自定义 provider：

```bash
rustcode config set provider custom
rustcode config set protocol anthropic
rustcode config set base_url https://api.example.com
rustcode config set model claude-3-5-sonnet-20241022
```

fallback：

```bash
rustcode config set fallback.enabled true
rustcode config set fallback.chain "deepseek:deepseek-chat,openai:gpt-4.1-mini"
```
