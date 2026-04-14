use crate::{
    config::{ApiProtocol, ApiProvider, FallbackTarget, Settings},
    onboarding::OnboardingDraft,
};
use egui::{Color32, RichText, Ui, Vec2};

use super::Theme;

pub struct OnboardingPanel {
    draft: OnboardingDraft,
    selected_fallback: usize,
    show_primary_api_key: bool,
    show_fallback_api_key: bool,
}

impl OnboardingPanel {
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            draft: OnboardingDraft::from_settings(settings),
            selected_fallback: 0,
            show_primary_api_key: false,
            show_fallback_api_key: false,
        }
    }

    pub fn apply_to_settings(&self, settings: &mut Settings) {
        self.draft.apply_to_settings(settings);
    }

    pub fn ui(&mut self, ui: &mut Ui, theme: &Theme) -> bool {
        let mut complete = false;

        ui.vertical_centered(|ui| {
            ui.add_space(12.0);
            ui.label(
                RichText::new("RustCode")
                    .size(28.0)
                    .strong()
                    .color(theme.primary_color()),
            );
            ui.label(
                RichText::new("Set your primary model and fallback chain before entering the app.")
                    .color(theme.muted_text_color()),
            );
        });

        ui.add_space(18.0);

        ui.columns(2, |columns| {
            self.render_primary_card(&mut columns[0], theme);
            self.render_fallback_card(&mut columns[1], theme);
        });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Summary").strong().color(theme.text_color()));
                for line in self.draft.summary_lines() {
                    ui.label(
                        RichText::new(line)
                            .color(theme.muted_text_color())
                            .size(11.0),
                    );
                }
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = egui::Button::new(
                    RichText::new("Save and Continue")
                        .strong()
                        .color(Color32::WHITE),
                )
                .fill(theme.primary_color())
                .min_size(Vec2::new(170.0, 40.0))
                .rounding(10.0);

                if ui.add(button).clicked() {
                    complete = true;
                }
            });
        });

        complete
    }

    fn render_primary_card(&mut self, ui: &mut Ui, theme: &Theme) {
        egui::Frame::none()
            .fill(theme.surface_color())
            .stroke(egui::Stroke::new(1.0, theme.border_color()))
            .rounding(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Primary Model")
                        .strong()
                        .color(theme.text_color()),
                );
                ui.add_space(8.0);

                let previous_provider = self.draft.provider;
                egui::ComboBox::from_id_source("onboarding_primary_provider")
                    .selected_text(self.draft.provider_label())
                    .show_ui(ui, |ui| {
                        for provider in providers() {
                            ui.selectable_value(
                                &mut self.draft.provider,
                                provider,
                                provider_label(provider),
                            );
                        }
                    });
                if self.draft.provider != previous_provider {
                    self.draft.prepare_for_provider_change(self.draft.provider);
                }

                if self.draft.provider == ApiProvider::Custom {
                    ui.add_space(8.0);
                    egui::ComboBox::from_id_source("onboarding_primary_protocol")
                        .selected_text(self.draft.protocol.as_str())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.draft.protocol,
                                ApiProtocol::OpenAi,
                                "openai",
                            );
                            ui.selectable_value(
                                &mut self.draft.protocol,
                                ApiProtocol::Anthropic,
                                "anthropic",
                            );
                        });
                    ui.add(
                        egui::TextEdit::singleline(
                            self.draft
                                .custom_provider_name
                                .get_or_insert_with(String::new),
                        )
                        .hint_text("Custom provider name"),
                    );
                }

                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.base_url)
                        .hint_text("Base URL")
                        .desired_width(ui.available_width()),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.model)
                        .hint_text("Model")
                        .desired_width(ui.available_width()),
                );

                ui.horizontal(|ui| {
                    let api_key = self.draft.api_key.get_or_insert_with(String::new);
                    ui.add(
                        egui::TextEdit::singleline(api_key)
                            .password(!self.show_primary_api_key)
                            .hint_text("API key")
                            .desired_width(ui.available_width() - 70.0),
                    );
                    if ui
                        .button(if self.show_primary_api_key {
                            "Hide"
                        } else {
                            "Show"
                        })
                        .clicked()
                    {
                        self.show_primary_api_key = !self.show_primary_api_key;
                    }
                });

                ui.label(
                    RichText::new("Leave API key empty for env-managed or local providers.")
                        .size(11.0)
                        .color(theme.muted_text_color()),
                );
            });
    }

    fn render_fallback_card(&mut self, ui: &mut Ui, theme: &Theme) {
        egui::Frame::none()
            .fill(theme.surface_color())
            .stroke(egui::Stroke::new(1.0, theme.border_color()))
            .rounding(12.0)
            .inner_margin(16.0)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.draft.fallback_enabled, "");
                    ui.label(
                        RichText::new("Fallback Chain")
                            .strong()
                            .color(theme.text_color()),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("+ Add").clicked() {
                            let index = self.draft.add_fallback_target(ApiProvider::OpenAI);
                            self.selected_fallback = index;
                        }
                    });
                });

                ui.label(
                    RichText::new("Fallbacks are tried in order when the primary target fails.")
                        .size(11.0)
                        .color(theme.muted_text_color()),
                );
                ui.add_space(8.0);

                let mut remove_index = None;
                for (index, target) in self.draft.fallback_chain.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let selected = self.selected_fallback == index;
                        let label = OnboardingDraft::fallback_target_label(target);
                        if ui.selectable_label(selected, label).clicked() {
                            self.selected_fallback = index;
                        }
                        if ui.small_button("Remove").clicked() {
                            remove_index = Some(index);
                        }
                    });
                }

                if let Some(index) = remove_index {
                    self.draft.fallback_chain.remove(index);
                    if self.selected_fallback >= self.draft.fallback_chain.len() {
                        self.selected_fallback = self.draft.fallback_chain.len().saturating_sub(1);
                    }
                    if self.draft.fallback_chain.is_empty() {
                        self.draft.fallback_enabled = false;
                    }
                }

                if let Some(target) = self.draft.fallback_chain.get_mut(self.selected_fallback) {
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    render_fallback_editor(ui, theme, target, &mut self.show_fallback_api_key);
                } else {
                    ui.add_space(12.0);
                    ui.label(
                        RichText::new("No fallback target selected.")
                            .color(theme.muted_text_color()),
                    );
                }
            });
    }
}

fn providers() -> [ApiProvider; 6] {
    [
        ApiProvider::DeepSeek,
        ApiProvider::OpenAI,
        ApiProvider::DashScope,
        ApiProvider::OpenRouter,
        ApiProvider::Ollama,
        ApiProvider::Custom,
    ]
}

fn provider_label(provider: ApiProvider) -> &'static str {
    match provider {
        ApiProvider::DeepSeek => "DeepSeek",
        ApiProvider::OpenAI => "OpenAI",
        ApiProvider::DashScope => "DashScope",
        ApiProvider::OpenRouter => "OpenRouter",
        ApiProvider::Ollama => "Ollama",
        ApiProvider::Custom => "Custom",
    }
}

fn fallback_defaults(provider: ApiProvider) -> FallbackTarget {
    let mut target = FallbackTarget {
        provider,
        protocol: None,
        custom_provider_name: None,
        api_key: None,
        base_url: None,
        model: provider.default_model().to_string(),
    };

    if provider == ApiProvider::Custom {
        target.protocol = Some(ApiProtocol::OpenAi);
        target.custom_provider_name = Some("custom".to_string());
        target.base_url = Some(provider.default_base_url().to_string());
    }

    target
}

fn render_fallback_editor(
    ui: &mut Ui,
    theme: &Theme,
    target: &mut FallbackTarget,
    show_fallback_api_key: &mut bool,
) {
    ui.label(
        RichText::new("Selected Fallback")
            .strong()
            .color(theme.text_color()),
    );
    ui.add_space(8.0);

    let previous_provider = target.provider;
    egui::ComboBox::from_id_source("onboarding_fallback_provider")
        .selected_text(provider_label(target.provider))
        .show_ui(ui, |ui| {
            for provider in providers() {
                ui.selectable_value(&mut target.provider, provider, provider_label(provider));
            }
        });
    if target.provider != previous_provider {
        *target = fallback_defaults(target.provider);
    }

    if target.provider == ApiProvider::Custom {
        egui::ComboBox::from_id_source("onboarding_fallback_protocol")
            .selected_text(target.protocol.unwrap_or(ApiProtocol::OpenAi).as_str())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut target.protocol, Some(ApiProtocol::OpenAi), "openai");
                ui.selectable_value(
                    &mut target.protocol,
                    Some(ApiProtocol::Anthropic),
                    "anthropic",
                );
            });
        ui.add(
            egui::TextEdit::singleline(target.custom_provider_name.get_or_insert_with(String::new))
                .hint_text("Custom provider name"),
        );
    }

    ui.add(
        egui::TextEdit::singleline(target.base_url.get_or_insert_with(String::new))
            .hint_text("Base URL")
            .desired_width(ui.available_width()),
    );
    ui.add(
        egui::TextEdit::singleline(&mut target.model)
            .hint_text("Model")
            .desired_width(ui.available_width()),
    );
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(target.api_key.get_or_insert_with(String::new))
                .password(!*show_fallback_api_key)
                .hint_text("API key")
                .desired_width(ui.available_width() - 70.0),
        );
        if ui
            .button(if *show_fallback_api_key {
                "Hide"
            } else {
                "Show"
            })
            .clicked()
        {
            *show_fallback_api_key = !*show_fallback_api_key;
        }
    });
}
