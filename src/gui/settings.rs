//! Settings Panel - Application settings UI

use crate::config::{ApiProvider, Settings};
use egui::{Color32, RichText, Ui, Vec2};

/// Settings panel state
pub struct SettingsPanel {
    pub current_section: SettingsSection,
    pub provider: String,
    pub protocol: String,
    pub custom_provider_name: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub fallback_enabled: bool,
    pub fallback_chain: String,
    pub theme: super::Theme,
    pub language: String,
    pub auto_save: bool,
    pub notifications: bool,
    pub telemetry: bool,
    pub show_api_key: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    General,
    Api,
    Appearance,
    Plugins,
    Advanced,
}

impl Default for SettingsPanel {
    fn default() -> Self {
        Self {
            current_section: SettingsSection::General,
            provider: "deepseek".to_string(),
            protocol: "openai".to_string(),
            custom_provider_name: String::new(),
            api_key: String::new(),
            base_url: "https://api.deepseek.com".to_string(),
            model: "deepseek-chat".to_string(),
            fallback_enabled: false,
            fallback_chain: "openai:gpt-4.1-mini".to_string(),
            theme: super::Theme::Dark,
            language: "en".to_string(),
            auto_save: true,
            notifications: true,
            telemetry: false,
            show_api_key: false,
        }
    }
}

impl SettingsPanel {
    pub fn from_settings(settings: &Settings, theme: super::Theme) -> Self {
        let fallback_chain = if settings.api.fallback.chain.is_empty() {
            String::new()
        } else {
            serde_json::to_string_pretty(&settings.api.fallback.chain).unwrap_or_default()
        };

        Self {
            current_section: SettingsSection::General,
            provider: settings.api.provider().as_str().to_string(),
            protocol: settings.api.protocol().as_str().to_string(),
            custom_provider_name: settings
                .api
                .custom_provider_name
                .clone()
                .unwrap_or_default(),
            api_key: settings.api.api_key.clone().unwrap_or_default(),
            base_url: settings.api.base_url.clone(),
            model: settings.model.clone(),
            fallback_enabled: settings.api.fallback.enabled,
            fallback_chain,
            theme,
            ..Default::default()
        }
    }

    pub fn apply_to_settings(&self, settings: &mut Settings) -> anyhow::Result<()> {
        settings.api.set_provider(&self.provider)?;

        if settings.api.provider == ApiProvider::Custom {
            settings.api.set_protocol(&self.protocol)?;
        }

        settings.api.custom_provider_name = if self.custom_provider_name.trim().is_empty() {
            None
        } else {
            Some(self.custom_provider_name.trim().to_string())
        };

        settings.api.api_key = if self.api_key.trim().is_empty() {
            None
        } else {
            Some(self.api_key.trim().to_string())
        };

        settings.api.base_url = if self.base_url.trim().is_empty() {
            settings.api.provider.default_base_url().to_string()
        } else {
            self.base_url.trim().to_string()
        };

        settings.model = if self.model.trim().is_empty() {
            settings.api.default_model().to_string()
        } else {
            self.model.trim().to_string()
        };

        settings.api.fallback.enabled = self.fallback_enabled;
        settings
            .api
            .set_fallback_chain_from_str(&self.fallback_chain)?;

        Ok(())
    }

    /// Render the settings panel
    pub fn ui(&mut self, ui: &mut Ui, theme: &super::Theme) -> bool {
        let mut save_requested = false;

        ui.horizontal(|ui| {
            // Left sidebar for sections
            ui.vertical(|ui| {
                ui.set_width(180.0);
                ui.set_min_height(ui.available_height());

                self.render_section_list(ui, theme);
            });

            ui.separator();

            // Right panel for settings content
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width());
                ui.set_min_height(ui.available_height());

                match self.current_section {
                    SettingsSection::General => self.render_general_settings(ui, theme),
                    SettingsSection::Api => self.render_api_settings(ui, theme),
                    SettingsSection::Appearance => self.render_appearance_settings(ui, theme),
                    SettingsSection::Plugins => self.render_plugin_settings(ui, theme),
                    SettingsSection::Advanced => self.render_advanced_settings(ui, theme),
                }

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Saved to ~/.rustcode/settings.json")
                            .color(theme.muted_text_color())
                            .size(11.0),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save_button = egui::Button::new(
                            RichText::new("Save Settings")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(theme.primary_color())
                        .min_size(Vec2::new(140.0, 36.0))
                        .rounding(8.0);

                        if ui.add(save_button).clicked() {
                            save_requested = true;
                        }
                    });
                });
            });
        });

        save_requested
    }

    fn apply_provider_defaults(&mut self) {
        if let Some(provider) = ApiProvider::parse(&self.provider) {
            self.base_url = provider.default_base_url().to_string();
            self.model = provider.default_model().to_string();

            if provider != ApiProvider::Custom {
                self.protocol = provider.default_protocol().as_str().to_string();
                self.custom_provider_name.clear();
            }
        }
    }

    fn render_section_list(&mut self, ui: &mut Ui, theme: &super::Theme) {
        let sections = vec![
            (SettingsSection::General, "⚙️", "General"),
            (SettingsSection::Api, "🔑", "API"),
            (SettingsSection::Appearance, "🎨", "Appearance"),
            (SettingsSection::Plugins, "🔌", "Plugins"),
            (SettingsSection::Advanced, "⚡", "Advanced"),
        ];

        for (section, icon, label) in sections {
            let is_selected = self.current_section == section;

            let button = egui::Button::new(RichText::new(format!("{} {}", icon, label)).color(
                if is_selected {
                    Color32::WHITE
                } else {
                    theme.text_color()
                },
            ))
            .fill(if is_selected {
                theme.primary_color()
            } else {
                theme.surface_color()
            })
            .min_size(Vec2::new(ui.available_width(), 40.0))
            .rounding(8.0);

            if ui.add(button).clicked() {
                self.current_section = section;
            }
            ui.add_space(4.0);
        }
    }

    fn render_general_settings(&mut self, ui: &mut Ui, theme: &super::Theme) {
        ui.heading(RichText::new("General Settings").color(theme.text_color()));
        ui.add_space(16.0);

        // Language selection
        ui.group(|ui| {
            ui.label(RichText::new("Language").strong().color(theme.text_color()));
            ui.add_space(4.0);

            egui::ComboBox::from_id_source("language")
                .selected_text(&self.language)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.language, "en".to_string(), "🇺🇸 English");
                    ui.selectable_value(&mut self.language, "zh".to_string(), "🇨🇳 中文");
                    ui.selectable_value(&mut self.language, "ja".to_string(), "🇯🇵 日本語");
                    ui.selectable_value(&mut self.language, "es".to_string(), "🇪🇸 Español");
                    ui.selectable_value(&mut self.language, "fr".to_string(), "🇫🇷 Français");
                    ui.selectable_value(&mut self.language, "de".to_string(), "🇩🇪 Deutsch");
                });
        });

        ui.add_space(16.0);

        // Auto-save toggle
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.auto_save, "");
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Auto-save conversations")
                        .strong()
                        .color(theme.text_color()),
                );
                ui.label(
                    RichText::new("Automatically save conversation history")
                        .color(theme.muted_text_color())
                        .size(11.0),
                );
            });
        });

        ui.add_space(8.0);

        // Notifications toggle
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.notifications, "");
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Enable notifications")
                        .strong()
                        .color(theme.text_color()),
                );
                ui.label(
                    RichText::new("Show notifications for important events")
                        .color(theme.muted_text_color())
                        .size(11.0),
                );
            });
        });

        ui.add_space(8.0);

        // Telemetry toggle
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.telemetry, "");
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Enable telemetry")
                        .strong()
                        .color(theme.text_color()),
                );
                ui.label(
                    RichText::new("Help improve RustCode by sharing anonymous usage data")
                        .color(theme.muted_text_color())
                        .size(11.0),
                );
            });
        });
    }

    fn render_api_settings(&mut self, ui: &mut Ui, theme: &super::Theme) {
        ui.heading(RichText::new("API Configuration").color(theme.text_color()));
        ui.add_space(16.0);

        ui.group(|ui| {
            let previous_provider = self.provider.clone();

            ui.label(RichText::new("Provider").strong().color(theme.text_color()));
            ui.add_space(4.0);

            egui::ComboBox::from_id_source("provider")
                .selected_text(&self.provider)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.provider, "deepseek".to_string(), "DeepSeek");
                    ui.selectable_value(&mut self.provider, "openai".to_string(), "OpenAI");
                    ui.selectable_value(&mut self.provider, "dashscope".to_string(), "DashScope");
                    ui.selectable_value(&mut self.provider, "openrouter".to_string(), "OpenRouter");
                    ui.selectable_value(&mut self.provider, "ollama".to_string(), "Ollama");
                    ui.selectable_value(&mut self.provider, "custom".to_string(), "Custom");
                });

            if self.provider != previous_provider {
                self.apply_provider_defaults();
            }

            if self.provider == "custom" {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Custom Provider Name")
                        .strong()
                        .color(theme.text_color()),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.custom_provider_name)
                        .hint_text("my-gateway")
                        .desired_width(ui.available_width()),
                );
            }
        });

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.label(RichText::new("Protocol").strong().color(theme.text_color()));
            ui.add_space(4.0);

            ui.add_enabled_ui(self.provider == "custom", |ui| {
                egui::ComboBox::from_id_source("protocol")
                    .selected_text(&self.protocol)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.protocol,
                            "openai".to_string(),
                            "OpenAI-compatible",
                        );
                        ui.selectable_value(
                            &mut self.protocol,
                            "anthropic".to_string(),
                            "Anthropic messages",
                        );
                    });
            });

            ui.label(
                RichText::new(if self.provider == "custom" {
                    "Custom providers can use either OpenAI or Anthropic wire format."
                } else {
                    "Preset providers use their default protocol automatically."
                })
                .color(theme.muted_text_color())
                .size(11.0),
            );
        });

        ui.add_space(16.0);

        // API Key
        ui.group(|ui| {
            ui.label(RichText::new("API Key").strong().color(theme.text_color()));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                let api_key_edit = egui::TextEdit::singleline(&mut self.api_key)
                    .password(!self.show_api_key)
                    .hint_text("Enter your API key")
                    .desired_width(ui.available_width() - 100.0);

                ui.add(api_key_edit);

                if ui
                    .button(if self.show_api_key { "Hide" } else { "Show" })
                    .clicked()
                {
                    self.show_api_key = !self.show_api_key;
                }
            });

            ui.label(
                RichText::new("Leave empty for local backends such as Ollama.")
                    .color(theme.muted_text_color())
                    .size(11.0),
            );
        });

        ui.add_space(16.0);

        // Base URL
        ui.group(|ui| {
            ui.label(RichText::new("Base URL").strong().color(theme.text_color()));
            ui.add_space(4.0);

            ui.add(
                egui::TextEdit::singleline(&mut self.base_url)
                    .hint_text("https://api.example.com")
                    .desired_width(ui.available_width()),
            );
        });

        ui.add_space(16.0);

        // Model selection
        ui.group(|ui| {
            ui.label(RichText::new("Model").strong().color(theme.text_color()));
            ui.add_space(4.0);

            ui.add(
                egui::TextEdit::singleline(&mut self.model)
                    .hint_text("gpt-4.1-mini")
                    .desired_width(ui.available_width()),
            );

            ui.label(
                RichText::new("Examples: deepseek-chat, gpt-4.1-mini, claude-3-5-sonnet-20241022")
                    .color(theme.muted_text_color())
                    .size(11.0),
            );
        });

        ui.add_space(16.0);

        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.fallback_enabled, "");
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Enable fallback chain")
                            .strong()
                            .color(theme.text_color()),
                    );
                    ui.label(
                        RichText::new(
                            "If the current model fails, RustCode will try the next target.",
                        )
                        .color(theme.muted_text_color())
                        .size(11.0),
                    );
                });
            });

            ui.add_space(8.0);
            ui.label(
                RichText::new("Fallback Chain")
                    .strong()
                    .color(theme.text_color()),
            );
            ui.add(
                egui::TextEdit::multiline(&mut self.fallback_chain)
                    .hint_text("[{\"provider\":\"openai\",\"model\":\"gpt-4.1-mini\"}]")
                    .desired_width(ui.available_width())
                    .desired_rows(3),
            );
        });
    }

    fn render_appearance_settings(&mut self, ui: &mut Ui, theme: &super::Theme) {
        ui.heading(RichText::new("Appearance").color(theme.text_color()));
        ui.add_space(16.0);

        // Theme selection
        ui.group(|ui| {
            ui.label(RichText::new("Theme").strong().color(theme.text_color()));
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                let themes = vec![
                    (super::Theme::Light, "☀️", "Light"),
                    (super::Theme::Dark, "🌙", "Dark"),
                    (super::Theme::System, "💻", "System"),
                ];

                for (t, icon, label) in themes {
                    let is_selected = self.theme == t;
                    let button = egui::Button::new(
                        RichText::new(format!("{} {}", icon, label)).color(if is_selected {
                            Color32::WHITE
                        } else {
                            theme.text_color()
                        }),
                    )
                    .fill(if is_selected {
                        theme.primary_color()
                    } else {
                        theme.surface_color()
                    })
                    .min_size(Vec2::new(100.0, 60.0))
                    .rounding(8.0);

                    if ui.add(button).clicked() {
                        self.theme = t;
                    }
                    ui.add_space(8.0);
                }
            });
        });

        ui.add_space(16.0);

        // Font size
        ui.group(|ui| {
            ui.label(
                RichText::new("Font Size")
                    .strong()
                    .color(theme.text_color()),
            );
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                if ui.button("A-").clicked() {
                    // Decrease font size
                }
                ui.label("100%");
                if ui.button("A+").clicked() {
                    // Increase font size
                }
            });
        });

        ui.add_space(16.0);

        // Compact mode
        ui.horizontal(|ui| {
            let mut compact_mode = false;
            ui.checkbox(&mut compact_mode, "");
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("Compact mode")
                        .strong()
                        .color(theme.text_color()),
                );
                ui.label(
                    RichText::new("Reduce padding and margins for a more compact view")
                        .color(theme.muted_text_color())
                        .size(11.0),
                );
            });
        });
    }

    fn render_plugin_settings(&mut self, ui: &mut Ui, theme: &super::Theme) {
        ui.heading(RichText::new("Plugin Settings").color(theme.text_color()));
        ui.add_space(16.0);

        // Installed plugins list
        let plugins = vec![
            ("File System", "1.0.0", "Access and manage files", true),
            (
                "Git Integration",
                "1.2.0",
                "Git commands and repository management",
                true,
            ),
            (
                "Code Analysis",
                "0.9.0",
                "Static code analysis tools",
                false,
            ),
            ("Terminal", "1.1.0", "Integrated terminal access", true),
        ];

        for (name, version, description, enabled) in plugins {
            let mut is_enabled = enabled;

            ui.group(|ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut is_enabled, "");

                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).strong().color(theme.text_color()));
                            ui.label(
                                RichText::new(format!("v{}", version))
                                    .color(theme.muted_text_color())
                                    .size(11.0),
                            );
                        });
                        ui.label(
                            RichText::new(description)
                                .color(theme.muted_text_color())
                                .size(11.0),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⚙️").clicked() {
                            // Open plugin settings
                        }
                        if ui.button("🗑️").clicked() {
                            // Uninstall plugin
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }

        ui.add_space(16.0);

        // Install new plugin button
        let install_button = egui::Button::new(
            RichText::new("➕ Install Plugin")
                .strong()
                .color(Color32::WHITE),
        )
        .fill(theme.primary_color())
        .min_size(Vec2::new(150.0, 36.0))
        .rounding(8.0);

        if ui.add(install_button).clicked() {
            // Open plugin marketplace
        }
    }

    fn render_advanced_settings(&mut self, ui: &mut Ui, theme: &super::Theme) {
        ui.heading(RichText::new("Advanced Settings").color(theme.text_color()));
        ui.add_space(16.0);

        // Cache settings
        ui.group(|ui| {
            ui.label(RichText::new("Cache").strong().color(theme.text_color()));
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label("Cache size: ");
                ui.label(RichText::new("125 MB").strong().color(theme.text_color()));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear Cache").clicked() {
                        // Clear cache
                    }
                });
            });
        });

        ui.add_space(16.0);

        // Data export/import
        ui.group(|ui| {
            ui.label(
                RichText::new("Data Management")
                    .strong()
                    .color(theme.text_color()),
            );
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("📥 Export Data").clicked() {
                    // Export data
                }
                if ui.button("📤 Import Data").clicked() {
                    // Import data
                }
            });
        });

        ui.add_space(16.0);

        // Reset settings
        ui.group(|ui| {
            ui.label(RichText::new("Reset").strong().color(theme.text_color()));
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.button("Reset Settings").clicked() {
                    // Reset to defaults
                }
                if ui.button("Clear All Data").clicked() {
                    // Clear all data
                }
            });
        });

        ui.add_space(16.0);

        // Developer options
        ui.collapsing(
            RichText::new("Developer Options").color(theme.text_color()),
            |ui| {
                let mut dev_mode = false;
                ui.checkbox(&mut dev_mode, "Enable developer mode");

                let mut debug_logging = false;
                ui.checkbox(&mut debug_logging, "Enable debug logging");

                let mut experimental_features = false;
                ui.checkbox(&mut experimental_features, "Enable experimental features");
            },
        );
    }

    /// Get the current theme
    pub fn theme(&self) -> super::Theme {
        self.theme
    }

    /// Set the theme
    pub fn set_theme(&mut self, theme: super::Theme) {
        self.theme = theme;
    }
}
