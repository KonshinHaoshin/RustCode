//! GUI Module - Desktop GUI using egui/eframe
//!
//! This module provides a native desktop GUI for RustCode
//! with a modern, responsive interface.

pub mod app;
pub mod chat;
pub mod onboarding;
pub mod settings;
pub mod sidebar;
pub mod theme;

pub use app::RustCodeApp;
pub use theme::Theme;
