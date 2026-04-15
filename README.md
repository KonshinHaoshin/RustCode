# RustCode (v0.1.3)

**RustCode** is a high-performance, multi-provider AI coding assistant built entirely in Rust. It serves as a native, local-first agent runtime designed for developers who demand speed, flexibility, and professional-grade tooling.

---

## 🚀 Core Features

### 1. Multi-Provider & Protocol Support
Native integration with the world's leading AI providers. Switch between them instantly or set up complex fallback chains.
- **Built-in Presets**: DeepSeek, OpenAI, Anthropic, xAI, Gemini, Dashscope (Aliyun), OpenRouter, Ollama.
- **Protocols**: OpenAI Chat, Anthropic Messages, and custom xAI-style Responses.
- **Custom Endpoints**: Full control over Base URL and API keys for any compatible proxy or local LLM.

### 2. Intelligent Agentic Runtime
A sophisticated core (`src/runtime`) that handles:
- **Multi-turn Loops**: Orchestrates complex reasoning and tool-use sequences.
- **Tool Integration**: Built-in system tools + **MCP (Model Context Protocol)** support for infinite extensibility.
- **Plan Mode**: Dedicated `/plan` mode for architectural design before code generation.
- **Permission Gate**: Fine-grained security controls (Always Allow / Ask / Read-only).

### 3. Triple-Interface Experience
- **✨ Pro Liquid GUI**: A stunning, high-density desktop application (Tauri + React).
  - Claude-inspired "Liquid" aesthetic with glassmorphism effects.
  - Multi-profile management system (stored in `~/.rustcode/profiles`).
  - Terminal-style Markdown rendering with macOS-style code blocks.
  - Smart Slash Command (`/`) popover.
- **🖥️ Immersive TUI**: A feature-rich Terminal UI (Ratatui).
  - Keyboard-driven scrolling (PageUp/Down, Home/End).
  - Resident ASCII mascot for a friendly terminal vibe.
  - Lightning-fast response and low resource footprint.
- **📟 Legacy REPL**: Traditional line-based interaction for simple queries.

---

## 📦 Installation

### Prerequisites
- [Rust](https://rustup.rs/) (Edition 2021)
- [Node.js & pnpm](https://pnpm.io/) (Only for GUI development)

### Build from Source
```bash
# Build the CLI & TUI
cargo build --release

# Install locally
cargo install --path .
```

### Build Desktop GUI (Tauri)
```bash
cd gui && pnpm install
cd ../src-tauri && cargo tauri build
```

---

## 🛠️ Configuration

RustCode uses a centralized configuration system located at `~/.rustcode/`.

- **Active Profile**: `~/.rustcode/active-profile`
- **Profile Details**: `~/.rustcode/profiles/[name].json`
- **Local Overrides**: `./rustcode/settings.local.json` (Project-specific)

### Quick Start
Simply run `rustcode` to enter the TUI and start the interactive onboarding flow:
```bash
rustcode
```

To launch the GUI (once built):
```bash
# Run from the GUI directory
cd gui && pnpm tauri:dev
```

---

## ⌨️ Slash Commands

| Command | Description |
| :--- | :--- |
| `/init` | Initialize project and generate `rustcode.md` |
| `/plan` | Toggle **Plan Mode** for architectural design |
| `/model` | Switch or view current AI model |
| `/clear` | Clear current conversation history |
| `/fix` | Analyze and fix issues in current code |
| `/review` | Deeply review code changes |
| `/status` | View system stats and token usage |
| `/mcp` | Manage Model Context Protocol servers |

---

## 🌐 Internationalization (i18n)

RustCode is built with global developers in mind.
- **Supported Languages**: English, 简体中文.
- **Switching**: Accessible via the **Settings / Voice & UI** tab in the GUI or through the onboarding flow.

---

## ⚖️ License & Credits

- **License**: MIT
- **Origin**: Proudly forked and enhanced from [lorryjovens-hub/claude-code-rust](https://github.com/lorryjovens-hub/claude-code-rust).
- **Goal**: To provide the fastest, most customizable Rust-based alternative to mainstream AI coding assistants.

---

*“Code at the speed of thought, powered by the safety of Rust.”*
