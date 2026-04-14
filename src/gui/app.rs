//! Main Application - GUI Application State and Logic

use crate::config::Settings;
use eframe::Frame;
use egui::{CentralPanel, Context, SidePanel, TopBottomPanel};

use super::{
    chat::ChatPanel,
    onboarding::OnboardingPanel,
    settings::SettingsPanel,
    sidebar::{Sidebar, Tab},
    Theme,
};

/// Main application state
pub struct RustCodeApp {
    settings: Settings,
    theme: Theme,
    sidebar: Sidebar,
    chat_panel: ChatPanel,
    settings_panel: SettingsPanel,
    onboarding_panel: OnboardingPanel,
    show_onboarding: bool,
    show_settings: bool,
    status_message: Option<String>,
    status_timer: Option<std::time::Instant>,
}

impl Default for RustCodeApp {
    fn default() -> Self {
        let settings = Settings::default();
        let theme = Theme::Dark;

        Self {
            settings_panel: SettingsPanel::from_settings(&settings, theme),
            onboarding_panel: OnboardingPanel::from_settings(&settings),
            settings,
            theme,
            sidebar: Sidebar::default(),
            chat_panel: ChatPanel::default(),
            show_onboarding: false,
            show_settings: false,
            status_message: None,
            status_timer: None,
        }
    }
}

impl RustCodeApp {
    /// Create a new application instance
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = Settings::load().unwrap_or_else(|_| Settings::default());
        let mut app = Self {
            settings_panel: SettingsPanel::from_settings(&settings, Theme::Dark),
            onboarding_panel: OnboardingPanel::from_settings(&settings),
            show_onboarding: settings.should_run_onboarding(),
            settings,
            ..Self::default()
        };

        // Apply theme
        app.theme.apply(&cc.egui_ctx);

        // Load custom fonts if needed
        Self::configure_fonts(&cc.egui_ctx);

        if app.show_onboarding {
            app.show_status("Complete onboarding to start using RustCode");
        } else {
            app.show_status("Ready");
        }

        app
    }

    /// Configure custom fonts
    fn configure_fonts(ctx: &Context) {
        let fonts = egui::FontDefinitions::default();

        // Add custom fonts here if needed
        // fonts.font_data.insert("my_font".to_owned(), ...);

        ctx.set_fonts(fonts);
    }

    /// Show a status message
    fn show_status(&mut self, message: impl Into<String>) {
        self.status_message = Some(message.into());
        self.status_timer = Some(std::time::Instant::now());
    }

    /// Update status timer
    fn update_status(&mut self) {
        if let Some(timer) = self.status_timer {
            if timer.elapsed().as_secs() > 3 {
                self.status_message = None;
                self.status_timer = None;
            }
        }
    }

    fn save_settings(&mut self) -> anyhow::Result<()> {
        self.settings_panel.apply_to_settings(&mut self.settings)?;
        self.theme = self.settings_panel.theme();
        self.settings.save()?;
        Ok(())
    }

    fn complete_onboarding(&mut self) -> anyhow::Result<()> {
        self.onboarding_panel.apply_to_settings(&mut self.settings);
        self.settings.mark_onboarding_complete();
        self.settings.save()?;
        self.settings_panel = SettingsPanel::from_settings(&self.settings, self.theme);
        self.show_onboarding = false;
        self.show_status("Onboarding complete");
        Ok(())
    }
}

impl eframe::App for RustCodeApp {
    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        let mut save_requested = false;
        let mut onboarding_complete = false;

        // Apply theme
        self.theme.apply(ctx);

        // Update status
        self.update_status();

        // Top panel - Title bar
        TopBottomPanel::top("top_panel")
            .exact_height(48.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Title
                    ui.heading(
                        egui::RichText::new("RustCode")
                            .color(self.theme.primary_color())
                            .size(20.0),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Window controls
                        if ui.button("➖").clicked() {
                            // Minimize window
                        }
                        if ui.button("⬜").clicked() {
                            // Maximize/restore window
                        }
                        if ui.button("✕").clicked() {
                            // Close window
                        }

                        ui.add_space(8.0);

                        // Settings button
                        let settings_text = if self.show_settings {
                            "✓ ⚙️"
                        } else {
                            "⚙️"
                        };
                        if ui.button(settings_text).clicked() {
                            self.show_settings = !self.show_settings;
                        }

                        ui.add_space(8.0);

                        // Theme toggle
                        let theme_icon = match self.theme {
                            Theme::Light => "☀️",
                            Theme::Dark => "🌙",
                            Theme::System => "💻",
                        };
                        if ui.button(theme_icon).clicked() {
                            self.theme = match self.theme {
                                Theme::Light => Theme::Dark,
                                Theme::Dark => Theme::System,
                                Theme::System => Theme::Light,
                            };
                            self.settings_panel.set_theme(self.theme);
                        }
                    });
                });
            });

        // Main content area
        if self.show_onboarding {
            CentralPanel::default().show(ctx, |ui| {
                onboarding_complete = self.onboarding_panel.ui(ui, &self.theme);
            });
        } else if self.show_settings {
            // Show settings panel
            CentralPanel::default().show(ctx, |ui| {
                save_requested = self.settings_panel.ui(ui, &self.theme);
            });
        } else {
            // Show main chat interface
            SidePanel::left("sidebar_panel")
                .resizable(true)
                .default_width(260.0)
                .min_width(200.0)
                .max_width(400.0)
                .show(ctx, |ui| {
                    self.sidebar.ui(ui, &self.theme);
                });

            CentralPanel::default().show(ctx, |ui| {
                match self.sidebar.selected_tab() {
                    Tab::Chat => {
                        self.chat_panel.ui(ui, &self.theme);
                    }
                    Tab::Settings => {
                        save_requested = self.settings_panel.ui(ui, &self.theme);
                    }
                    _ => {
                        // Other tabs - show placeholder
                        ui.vertical_centered(|ui| {
                            ui.add_space(ui.available_height() / 2.0 - 50.0);
                            ui.heading(
                                egui::RichText::new("Coming Soon")
                                    .color(self.theme.muted_text_color())
                                    .size(24.0),
                            );
                            ui.label(
                                egui::RichText::new("This feature is under development")
                                    .color(self.theme.muted_text_color()),
                            );
                        });
                    }
                }
            });
        }

        if save_requested {
            match self.save_settings() {
                Ok(()) => self.show_status("Settings saved"),
                Err(error) => self.show_status(format!("Failed to save settings: {}", error)),
            }
        }

        if onboarding_complete {
            match self.complete_onboarding() {
                Ok(()) => {}
                Err(error) => self.show_status(format!("Failed to save onboarding: {}", error)),
            }
        }

        // Bottom panel - Status bar
        TopBottomPanel::bottom("bottom_panel")
            .exact_height(28.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    // Status message
                    if let Some(ref message) = self.status_message {
                        ui.label(
                            egui::RichText::new(message)
                                .color(self.theme.info_color())
                                .size(11.0),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new("Ready")
                                .color(self.theme.muted_text_color())
                                .size(11.0),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Version info
                        ui.label(
                            egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                                .color(self.theme.muted_text_color())
                                .size(11.0),
                        );

                        ui.add_space(16.0);

                        // Connection status
                        ui.label(
                            egui::RichText::new("● Connected")
                                .color(self.theme.success_color())
                                .size(11.0),
                        );
                    });
                });
            });
    }

    fn on_exit(&mut self, _ctx: Option<&eframe::glow::Context>) {
        let _ = self.save_settings();
    }
}

/// Run the GUI application
pub fn run_gui() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "RustCode",
        options,
        Box::new(|cc| Ok(Box::new(RustCodeApp::new(cc)))),
    )
}
