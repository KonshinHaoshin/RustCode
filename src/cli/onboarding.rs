//! Interactive line-based onboarding flow for provider and fallback configuration.

use crate::config::{ApiProtocol, ApiProvider, FallbackTarget, Settings};
use crate::onboarding::OnboardingDraft;
use anyhow::Context;
use std::io::{self, Write};

pub fn run_config_onboarding() -> anyhow::Result<()> {
    let _ = crossterm::terminal::disable_raw_mode();
    let mut settings = Settings::load()?;
    let mut draft = OnboardingDraft::from_settings(&settings);

    println!();
    println!("RustCode model onboarding");
    println!("Configure the primary model first, then optionally add fallback targets.");
    println!("Press Enter to accept the default shown in brackets.");
    println!();

    let provider = prompt_provider("Primary provider", draft.provider)?;
    draft.prepare_for_provider_change(provider);

    if draft.provider == ApiProvider::Custom {
        draft.set_protocol(prompt_protocol("Primary protocol", draft.protocol)?);
        draft.set_custom_provider_name(prompt_line(
            "Custom provider name",
            draft.custom_provider_name.as_deref().unwrap_or("custom"),
        )?);
        draft.set_base_url(prompt_line("Base URL", &draft.base_url)?);
    } else {
        draft.set_protocol(draft.provider.default_protocol());
        draft.set_custom_provider_name(String::new());
        draft.set_base_url(prompt_line("Base URL", draft.provider.default_base_url())?);
    }

    draft.set_model(prompt_line("Model", &draft.model)?);
    draft.set_api_key(prompt_optional_secret(
        "API key",
        draft.api_key.as_ref(),
        draft.provider == ApiProvider::Ollama,
    )?);

    let keep_existing_fallbacks = !draft.fallback_chain.is_empty()
        && prompt_yes_no("Start from the current fallback chain?", true)?;

    let existing_fallbacks = if keep_existing_fallbacks {
        draft.fallback_chain.clone()
    } else {
        Vec::new()
    };

    draft.fallback_chain = prompt_fallback_chain(existing_fallbacks)?;
    draft.fallback_enabled = !draft.fallback_chain.is_empty();

    print_summary(&draft);

    if !prompt_yes_no("Save this configuration?", true)? {
        println!("Configuration was not changed.");
        return Ok(());
    }

    draft.apply_to_settings(&mut settings);
    settings.mark_onboarding_complete();
    settings.save()?;
    println!("Saved to ~/.rustcode/settings.json");
    Ok(())
}

fn prompt_fallback_chain(existing: Vec<FallbackTarget>) -> anyhow::Result<Vec<FallbackTarget>> {
    let mut chain = Vec::new();

    if !existing.is_empty() {
        println!();
        println!("Existing fallback targets:");
        for (index, target) in existing.iter().enumerate() {
            println!(
                "  {}. {}",
                index + 1,
                OnboardingDraft::fallback_target_label(target)
            );
        }
        println!();
    }

    if !prompt_yes_no("Configure fallback targets?", !existing.is_empty())? {
        return Ok(Vec::new());
    }

    for (index, target) in existing.into_iter().enumerate() {
        if prompt_yes_no(
            &format!(
                "Keep fallback target {} ({})?",
                index + 1,
                OnboardingDraft::fallback_target_label(&target)
            ),
            true,
        )? {
            chain.push(target);
        }
    }

    loop {
        if !prompt_yes_no("Add another fallback target?", chain.is_empty())? {
            break;
        }

        let provider = prompt_provider("Fallback provider", ApiProvider::OpenAI)?;
        let mut draft = OnboardingDraft {
            provider,
            protocol: if provider == ApiProvider::Custom {
                ApiProtocol::OpenAi
            } else {
                provider.default_protocol()
            },
            custom_provider_name: None,
            api_key: None,
            base_url: provider.default_base_url().to_string(),
            model: provider.default_model().to_string(),
            fallback_enabled: false,
            fallback_chain: Vec::new(),
        };

        if provider == ApiProvider::Custom {
            draft.set_protocol(prompt_protocol("Fallback protocol", ApiProtocol::OpenAi)?);
            draft.set_custom_provider_name(prompt_line("Fallback custom provider name", "custom")?);
            draft.set_base_url(prompt_line(
                "Fallback base URL",
                provider.default_base_url(),
            )?);
        } else if prompt_yes_no("Override the default base URL?", false)? {
            draft.set_base_url(prompt_line(
                "Fallback base URL",
                provider.default_base_url(),
            )?);
        }

        draft.set_model(prompt_line("Fallback model", provider.default_model())?);
        draft.set_api_key(prompt_optional_secret(
            "Fallback API key",
            None,
            provider == ApiProvider::Ollama,
        )?);

        chain.push(FallbackTarget {
            provider: draft.provider,
            protocol: (draft.provider == ApiProvider::Custom).then_some(draft.protocol),
            custom_provider_name: draft.custom_provider_name,
            api_key: draft.api_key,
            base_url: if draft.base_url.trim() == provider.default_base_url() {
                None
            } else {
                Some(draft.base_url)
            },
            model: draft.model,
        });
    }

    Ok(chain)
}

fn print_summary(draft: &OnboardingDraft) {
    println!();
    println!("Summary");
    for line in draft.summary_lines() {
        println!("  {}", line);
    }
    println!();
}

fn prompt_provider(label: &str, default: ApiProvider) -> anyhow::Result<ApiProvider> {
    let providers = [
        ApiProvider::DeepSeek,
        ApiProvider::OpenAI,
        ApiProvider::Anthropic,
        ApiProvider::XAI,
        ApiProvider::Gemini,
        ApiProvider::DashScope,
        ApiProvider::OpenRouter,
        ApiProvider::Ollama,
        ApiProvider::Custom,
    ];

    loop {
        println!("{}:", label);
        for (index, provider) in providers.iter().enumerate() {
            let marker = if *provider == default {
                " (default)"
            } else {
                ""
            };
            println!("  {}. {}{}", index + 1, provider.as_str(), marker);
        }

        let input = prompt_line("Choose a provider by number or name", default.as_str())?;
        if let Ok(index) = input.parse::<usize>() {
            if let Some(provider) = providers.get(index.saturating_sub(1)) {
                return Ok(*provider);
            }
        }

        if let Some(provider) = ApiProvider::parse(&input) {
            return Ok(provider);
        }

        println!("Invalid provider selection: {}", input);
        println!();
    }
}

fn prompt_protocol(label: &str, default: ApiProtocol) -> anyhow::Result<ApiProtocol> {
    loop {
        let input = prompt_line(label, default.as_str())?;
        if let Some(protocol) = ApiProtocol::parse(&input) {
            return Ok(protocol);
        }

        println!(
            "Invalid protocol: {}. Use openai, anthropic, or responses.",
            input
        );
    }
}

fn prompt_optional_secret(
    label: &str,
    existing: Option<&String>,
    allow_blank: bool,
) -> anyhow::Result<Option<String>> {
    loop {
        let prompt = match (existing, allow_blank) {
            (Some(_), true) => format!(
                "{} [press Enter to keep current, type '-' to clear, blank allowed]",
                label
            ),
            (Some(_), false) => {
                format!("{} [press Enter to keep current, type '-' to clear]", label)
            }
            (None, true) => format!("{} [leave blank to skip]", label),
            (None, false) => format!("{} [leave blank to use env var if available]", label),
        };

        let input = prompt_raw_line(&prompt)?;
        let trimmed = input.trim();

        if trimmed == "-" {
            return Ok(None);
        }

        if trimmed.is_empty() {
            if let Some(current) = existing {
                return Ok(Some(current.clone()));
            }

            return Ok(None);
        }

        return Ok(Some(trimmed.to_string()));
    }
}

fn prompt_yes_no(label: &str, default: bool) -> anyhow::Result<bool> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };

    loop {
        let input = prompt_raw_line(&format!("{} {}", label, suffix))?;
        let trimmed = input.trim().to_ascii_lowercase();

        if trimmed.is_empty() {
            return Ok(default);
        }

        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("Please answer y or n."),
        }
    }
}

fn prompt_line(label: &str, default: &str) -> anyhow::Result<String> {
    let input = prompt_raw_line(&format!("{} [{}]", label, default))?;
    if input.trim().is_empty() {
        Ok(default.to_string())
    } else {
        Ok(input.trim().to_string())
    }
}

fn prompt_raw_line(label: &str) -> anyhow::Result<String> {
    print!("{}: ", label);
    io::stdout().flush().context("failed to flush stdout")?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read user input")?;

    if input.contains('\u{3}') {
        return Err(anyhow::anyhow!("onboarding cancelled"));
    }

    Ok(input.trim_end_matches(['\r', '\n']).to_string())
}
