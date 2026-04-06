//! RustCode - High-performance multi-provider coding CLI
//!
//! A native Rust coding assistant, featuring:
//! - Async-first architecture with Tokio
//! - Native terminal UI with Ratatui
//! - MCP protocol support
//! - Voice input support
//! - Memory management and team sync
//! - Plugin system
//! - SSH connection support
//! - Remote execution
//! - Project initialization
//! - WebAssembly support for browser environments
//! - Native GUI with egui/eframe
//! - Plugin marketplace web interface
//! - Multi-language i18n support

pub mod advanced;
pub mod api;
pub mod cli;
pub mod compact;
pub mod config;
pub mod input;
pub mod mcp;
pub mod memory;
pub mod onboarding;
pub mod permissions;
pub mod plugins;
pub mod runtime;
pub mod services;
pub mod session;
pub mod state;
pub mod terminal;
pub mod tools;
pub mod tools_runtime;
pub mod utils;
pub mod voice;

// Feature-gated modules
#[cfg(feature = "gui-egui")]
pub mod gui;
#[cfg(feature = "i18n")]
pub mod i18n;
#[cfg(feature = "wasm")]
pub mod wasm;
#[cfg(feature = "web")]
pub mod web;

pub use advanced::{ProjectInitializer, RemoteExecutor, SshClient};
pub use api::{AnthropicClient, ApiClient, ChatMessage};
pub use cli::Cli;
pub use config::Settings;
pub use mcp::McpManager;
pub use memory::MemoryManager;
pub use onboarding::OnboardingDraft;
pub use permissions::{PermissionMode, PermissionsSettings};
pub use plugins::PluginManager;
pub use runtime::{QueryEngine, RuntimeMessage, RuntimeRole};
pub use state::AppState;
pub use tools::ToolRegistry;
pub use voice::VoiceInput;

// Feature-gated re-exports
#[cfg(feature = "gui-egui")]
pub use gui::RustCodeApp;
#[cfg(feature = "i18n")]
pub use i18n::Translator;
#[cfg(feature = "wasm")]
pub use wasm::ClaudeCodeWasm;
#[cfg(feature = "web")]
pub use web::WebServer;
