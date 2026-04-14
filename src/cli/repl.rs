//! REPL Module - Interactive Read-Eval-Print Loop

use crate::agents_runtime::run_agent_with_parent_history;
use crate::compact::CompactService;
use crate::input::commands::init::{run_init, InitMode};
use crate::input::commands::local::{
    run_diff_command, run_doctor_command, run_mcp_command, run_plugin_command, run_skills_command,
};
use crate::input::commands::plan::{plan_help_text, render_session_plan};
use crate::input::{
    format_help_text, format_status_text, InputProcessor, LocalCommand, PlanSlashAction,
    ProcessedInput,
};
use crate::runtime::{QueryEngine, RuntimeMessage, RuntimeRole};
use crate::services::AgentsService;
use crate::session::SessionPlan;
use crate::state::AppState;
use std::io::{self, BufRead, Write};

pub struct Repl {
    state: AppState,
    conversation_history: Vec<RuntimeMessage>,
    plan_mode: bool,
    active_plan: Option<SessionPlan>,
    last_usage_total: Option<usize>,
}

impl Repl {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            conversation_history: Vec::new(),
            plan_mode: false,
            active_plan: None,
            last_usage_total: None,
        }
    }

    pub fn start(&mut self, initial_prompt: Option<String>) -> anyhow::Result<()> {
        self.print_welcome();

        if let Some(prompt) = initial_prompt {
            self.process_input(&prompt)?;
        }

        let stdin = io::stdin();
        let mut stdout = io::stdout();

        loop {
            print!("> ");
            stdout.flush()?;

            let mut input = String::new();
            stdin.lock().read_line(&mut input)?;
            let input = input.trim();

            if input.is_empty() {
                continue;
            }

            match input {
                "exit" | "quit" | ".exit" => {
                    println!("\n👋 再见！");
                    break;
                }
                "help" | ".help" => self.execute_local_command(LocalCommand::Help)?,
                "status" | ".status" => self.execute_local_command(LocalCommand::Status)?,
                "clear" | ".clear" => self.clear_screen(),
                "history" | ".history" => self.print_history(),
                "reset" | ".reset" => self.execute_local_command(LocalCommand::Clear)?,
                "config" | ".config" => self.print_config(),
                _ => self.process_input(input)?,
            }
        }

        Ok(())
    }

    fn print_welcome(&self) {
        println!();
        println!("╔═══════════════════════════════════════════════════════════╗");
        println!("║           🔵 RustCode - 蓝色高性能编码助手              ║");
        println!("╚═══════════════════════════════════════════════════════════╝");
        println!();
        println!("  🚀 性能优势:");
        println!("  • 启动速度提升 85% 以上");
        println!("  • 内存占用减少 60%");
        println!("  • 响应速度提升 40%");
        println!("  • 资源利用率优化 70%");
        println!();
        println!("  Provider: {}", self.state.settings.api.provider_label());
        println!("  模型: {}", self.state.settings.model);
        println!("  输入 'help' 查看帮助, 'exit' 退出");
        println!();
    }

    fn process_input(&mut self, input: &str) -> anyhow::Result<()> {
        let prompt = match InputProcessor::new().process(input) {
            ProcessedInput::LocalCommand(command) => return self.execute_local_command(command),
            ProcessedInput::Error(message) => {
                println!("{}", message);
                println!();
                return Ok(());
            }
            ProcessedInput::Prompt(prompt) => prompt,
        };

        let engine = QueryEngine::new(self.state.settings.clone());

        println!();
        print!("🤖 ");
        io::stdout().flush()?;

        if self.plan_mode {
            return self.submit_plan_prompt(&prompt);
        }

        let messages = self.conversation_history.clone();
        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(engine.submit_text_turn(&messages, prompt))
        });

        match response {
            Ok(response) => {
                if response.status == crate::runtime::TurnStatus::AwaitingApproval {
                    println!(
                        "This action requires approval. Use the TUI to approve tool execution."
                    );
                    println!();
                    self.conversation_history = response.history;
                    return Ok(());
                }

                if let Some(content) = response.assistant_text() {
                    if !content.is_empty() {
                        println!("{}", content);
                        println!();
                    }
                }

                self.conversation_history = response.history;
                self.last_usage_total = response.usage.as_ref().map(|usage| usage.total_tokens);

                if let Some(usage) = response.usage {
                    println!(
                        "📊 Tokens: {} 提示 + {} 生成 = {} 总计",
                        usage.prompt_tokens, usage.completion_tokens, usage.total_tokens
                    );
                }
            }
            Err(error) => {
                self.conversation_history.push(RuntimeMessage::user(input));
                println!("请求失败: {}", error);
            }
        }

        Ok(())
    }

    fn submit_plan_prompt(&mut self, prompt: &str) -> anyhow::Result<()> {
        let Some(agent) = AgentsService::builtin_definition_by_name("plan") else {
            println!("Plan agent is not available.");
            println!();
            return Ok(());
        };

        let messages = self.conversation_history.clone();
        let prompt_text = prompt.to_string();
        let settings = self.state.settings.clone();
        let project_root = Some(self.state.settings.working_dir.clone());
        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(run_agent_with_parent_history(
                settings,
                project_root,
                agent,
                &messages,
                prompt_text.clone(),
                None,
            ))
        });

        match response {
            Ok(content) => {
                if !content.is_empty() {
                    println!("{}", content);
                    println!();
                }
                self.active_plan = Some(content.clone());
                self.conversation_history
                    .push(RuntimeMessage::user(prompt.to_string()));
                self.conversation_history
                    .push(RuntimeMessage::assistant(content));
                self.last_usage_total = None;
            }
            Err(error) => {
                self.conversation_history
                    .push(RuntimeMessage::user(prompt.to_string()));
                println!("请求失败: {}", error);
            }
        }

        Ok(())
    }

    fn print_help(&self) {
        println!();
        println!("{}", format_help_text());
        println!();
        println!("Legacy REPL commands:");
        println!("  history, .history  - 显示对话历史");
        println!("  config, .config    - 显示配置信息");
        println!("  clear, .clear      - 清屏");
        println!("  exit, .exit        - 退出 REPL");
        println!();
    }

    fn print_status(&self) {
        println!();
        println!(
            "{}",
            format_status_text(
                &self.state.settings,
                None,
                self.conversation_history.len(),
                false,
                self.last_usage_total,
                self.plan_mode,
                self.active_plan.as_ref(),
            )
        );
        println!();
    }

    fn print_history(&self) {
        println!();
        if self.conversation_history.is_empty() {
            println!("📜 对话历史为空");
        } else {
            println!("📜 对话历史 ({} 条消息):", self.conversation_history.len());
            for (i, msg) in self.conversation_history.iter().enumerate() {
                let role = match msg.role {
                    RuntimeRole::User => "👤 用户",
                    RuntimeRole::Assistant => "🤖 助手",
                    RuntimeRole::System => "⚙️ 系统",
                    RuntimeRole::Tool => "🛠️ 工具",
                };
                let preview: String = msg.content.chars().take(50).collect();
                let suffix = if msg.content.len() > 50 { "..." } else { "" };
                println!("  {}. {}: {}{}", i + 1, role, preview, suffix);
            }
        }
        println!();
    }

    fn print_config(&self) {
        println!();
        println!("⚙️ 配置信息:");
        println!(
            "{}",
            serde_json::to_string_pretty(&self.state.settings).unwrap_or_default()
        );
        println!();
    }

    fn reset_conversation(&mut self) {
        self.conversation_history.clear();
        self.plan_mode = false;
        self.active_plan = None;
        self.last_usage_total = None;
        println!();
        println!("🔄 对话已重置");
        println!();
    }

    fn clear_screen(&self) {
        print!("\x1B[2J\x1B[1;1H");
        io::stdout().flush().ok();
    }

    fn print_result(&self, result: anyhow::Result<String>, failure: &str) {
        println!();
        match result {
            Ok(message) => println!("{}", message),
            Err(error) => println!("{} {}", failure, error),
        }
        println!();
    }

    fn print_async_result<F>(&self, future: F, failure: &str) -> anyhow::Result<()>
    where
        F: std::future::Future<Output = anyhow::Result<String>>,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        self.print_result(runtime.block_on(future), failure);
        Ok(())
    }

    fn handle_plan_command(&mut self, action: PlanSlashAction) -> anyhow::Result<()> {
        match action {
            PlanSlashAction::Enter { prompt } => {
                if self.plan_mode {
                    self.show_plan();
                } else {
                    self.plan_mode = true;
                    println!();
                    println!("Plan mode enabled.");
                    println!();
                    if let Some(prompt) = prompt {
                        self.process_input(&prompt)?;
                    }
                }
            }
            PlanSlashAction::Show => self.show_plan(),
            PlanSlashAction::Open => {
                println!();
                println!("Opening the current plan in an external editor is not implemented.");
                println!();
            }
            PlanSlashAction::Exit => {
                self.plan_mode = false;
                println!();
                println!("Plan mode disabled.");
                println!();
            }
        }
        Ok(())
    }

    fn show_plan(&self) {
        println!();
        match self.active_plan.as_ref() {
            Some(plan) => println!("{}", render_session_plan(plan)),
            None => println!("{}", plan_help_text()),
        }
        println!();
    }

    fn execute_local_command(&mut self, command: LocalCommand) -> anyhow::Result<()> {
        match command {
            LocalCommand::Help => self.print_help(),
            LocalCommand::Clear => self.reset_conversation(),
            LocalCommand::Diff { full } => self.print_result(
                run_diff_command(Some(&self.state.settings.working_dir), full),
                "Diff failed.",
            ),
            LocalCommand::Doctor => {
                self.print_result(run_doctor_command(&self.state.settings), "Doctor failed.")
            }
            LocalCommand::Init { force, append } => {
                println!();
                let mode = if append {
                    InitMode::Append
                } else if force {
                    InitMode::Force
                } else {
                    InitMode::Create
                };
                let cwd = std::env::current_dir()?;
                let outcome = run_init(&cwd, mode)?;
                println!("{}", outcome.message);
                println!();
            }
            LocalCommand::Branch { .. } => {
                println!();
                println!("/branch is only supported in the TUI right now.");
                println!();
            }
            LocalCommand::Compact { instructions } => self.compact_history(instructions)?,
            LocalCommand::Mcp { action } => {
                self.print_async_result(run_mcp_command(&action), "MCP command failed.")?
            }
            LocalCommand::Permissions => {
                println!();
                println!("/permissions is only interactive in the TUI.");
                println!();
            }
            LocalCommand::Model { model } => {
                println!();
                if let Some(model) = model {
                    self.state.settings.model = model.clone();
                    println!(
                        "Active model changed to {}/{}.",
                        self.state.settings.api.provider_label(),
                        model
                    );
                } else {
                    println!(
                        "Active model: {}/{}.",
                        self.state.settings.api.provider_label(),
                        self.state.settings.model
                    );
                }
                println!();
            }
            LocalCommand::Plan { action } => self.handle_plan_command(action)?,
            LocalCommand::Plugin { action } => {
                self.print_async_result(run_plugin_command(&action), "Plugin command failed.")?
            }
            LocalCommand::Status => self.print_status(),
            LocalCommand::Skills { action } => {
                self.print_result(run_skills_command(&action), "Skills command failed.")
            }
            LocalCommand::Rewind { .. } => {
                println!();
                println!("/rewind is only supported in the TUI right now.");
                println!();
            }
            LocalCommand::Resume { .. } => {
                println!();
                println!("/resume is only supported in the TUI right now.");
                println!();
            }
        }

        Ok(())
    }

    fn compact_history(&mut self, instructions: Option<String>) -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let outcome = runtime.block_on(async {
            CompactService::new(self.state.settings.clone())
                .compact_history(&self.conversation_history, instructions.as_deref())
                .await
        })?;
        self.conversation_history = outcome.history;
        println!();
        println!("Conversation compacted.");
        println!();
        Ok(())
    }
}
