//! REPL Module - Interactive Read-Eval-Print Loop

use crate::compact::CompactService;
use crate::input::{
    format_help_text, format_status_text, InputProcessor, LocalCommand, ProcessedInput,
};
use crate::runtime::{QueryEngine, RuntimeMessage, RuntimeRole};
use crate::state::AppState;
use std::io::{self, BufRead, Write};

pub struct Repl {
    state: AppState,
    conversation_history: Vec<RuntimeMessage>,
    last_usage_total: Option<usize>,
}

impl Repl {
    pub fn new(state: AppState) -> Self {
        Self {
            state,
            conversation_history: Vec::new(),
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
        match InputProcessor::new().process(input) {
            ProcessedInput::LocalCommand(command) => return self.execute_local_command(command),
            ProcessedInput::Prompt(_) => {}
        }

        let engine = QueryEngine::new(self.state.settings.clone());

        println!();
        print!("🤖 ");
        io::stdout().flush()?;

        let messages = self.conversation_history.clone();
        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(engine.submit_text_turn(&messages, input))
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
        self.last_usage_total = None;
        println!();
        println!("🔄 对话已重置");
        println!();
    }

    fn clear_screen(&self) {
        print!("\x1B[2J\x1B[1;1H");
        io::stdout().flush().ok();
    }

    fn execute_local_command(&mut self, command: LocalCommand) -> anyhow::Result<()> {
        match command {
            LocalCommand::Help => self.print_help(),
            LocalCommand::Clear => self.reset_conversation(),
            LocalCommand::Branch { .. } => {
                println!();
                println!("/branch is only supported in the TUI right now.");
                println!();
            }
            LocalCommand::Compact { instructions } => self.compact_history(instructions)?,
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
            LocalCommand::Status => self.print_status(),
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
