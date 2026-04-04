# RustCode Quickstart

## Build

```bash
cargo build --release
```

## Install

```bash
cargo install --path .
```

## Verify

```bash
rustcode --help
rustcode --version
```

## Configure a provider

First run:

```bash
rustcode
```

This opens the full-screen TUI and automatically starts onboarding if configuration is incomplete.

Interactive onboarding:

```bash
rustcode config onboard
```

Manual commands:

```bash
rustcode config set provider deepseek
rustcode config set api_key "your-api-key"
rustcode config set model deepseek-chat
```

## Configure a custom provider

```bash
rustcode config set provider custom
rustcode config set protocol openai
rustcode config set custom_provider_name my-gateway
rustcode config set base_url https://api.example.com
rustcode config set model custom-model
```

## Enable fallback

```bash
rustcode config set fallback.enabled true
rustcode config set fallback.chain "deepseek:deepseek-chat,openai:gpt-4.1-mini"
```

## Run

```bash
rustcode query --prompt "Hello"
rustcode
rustcode tui
rustcode repl
```

`rustcode` and `rustcode tui` use the new full-screen TUI.
`rustcode repl` keeps the legacy line-based REPL.
