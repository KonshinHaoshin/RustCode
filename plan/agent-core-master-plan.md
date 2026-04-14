# RustCode Agent Core Master Plan

## Goal

将 `rustcode` 从多模型聊天客户端逐步重构为接近 `claude-code-rev-main` 的本地 agent runtime，同时保留现有多 provider、custom provider、fallback 能力。

## Scope

- 优先复刻本地 agent core
- 先完成 runtime 内核，再做 TUI 1:1 对齐
- `claude-code-rev-main/src` 作为主要对照源码
- `claude-code-main (2)` 仅作为插件和文档补充参考

## Phases

### Phase 0

- 落盘总计划和阶段文档
- 建立 runtime 数据模型映射文档

Status: completed

### Phase 1

- 建立最小 `runtime::QueryEngine`
- 统一 TUI、REPL、`query`、`run_agent` 的纯文本请求路径
- 引入运行时消息模型，不接工具执行

Status: completed

### Phase 2

- 工具注册和工具执行编排
- provider 侧 tool schema 适配
- `tool_use -> tool_result -> continue loop`

Status: completed

### Phase 3

- 权限系统
- transcript/session 持久化
- 可恢复会话

Status: completed

### Phase 4

- 输入预处理
- slash commands
- compact / token budget

Status: completed

### Phase 5

- Claude Code 风格 TUI 对齐
- transcript-first layout
- tool progress / permission dialog / copy mode

Status: completed

### Phase 6

- agent extensions
- 子 agent runtime
- 更细粒度的 agent-specific context

Status: completed

### Phase 7

- session fork / replay / rewind
- transcript message identity
- file-history backed rewind

Status: completed

### Phase 8

- Tauri desktop migration
- React/Vite transcript-first desktop shell
- runtime/session/event bridge for desktop GUI

Status: in progress

## Done Criteria

- 主交互入口不再直接调用 `ApiClient.chat`
- 统一由 runtime 编排纯文本、工具、权限、session、compact
- TUI 可以承载 agent runtime 全流程
- 多 provider/custom provider/fallback 继续可用

## Update Rule

每完成一个 phase，必须同步更新：

- 本文件状态
- 对应 `plan/phase-xx-*.md`

## Latest Update

- Phase 0 completed: master plan, phase logs, runtime mapping doc landed under `plan/`
- Phase 0 completed: `.gitignore` updated to allow tracking `plan/*.md`
- Phase 1 completed: added `src/runtime` foundation and routed TUI, REPL, `query`, and `run_agent` through `QueryEngine`
- Phase 2 in progress: added `src/tools_runtime`, canonical runtime tool call/result model, and an OpenAI-style tool loop in `QueryEngine`
- Phase 2 in progress: builtin tools are now exposed to the OpenAI/custom-open_ai request path, and `execute_command` was fixed for Windows shells
- Phase 2 in progress: Anthropic now has baseline tool-use/tool-result protocol mapping plus provider-aware tool target selection
- Phase 2 completed: external MCP stdio tool discovery/execution is integrated into runtime via namespaced tools
- Phase 3 in progress: runtime tool execution now passes through a permissions gate backed by `settings.permissions`
- Phase 3 in progress: added project-local `./rustcode/settings.local.json` merge/load and Claude Code-style workspace-local permission persistence
- Phase 3 in progress: `ask` now yields `AwaitingApproval`, `QueryEngine` can resume after approval, and the TUI exposes inline `allow once` / `deny once` / `always allow` / `always deny`
- Phase 3 in progress: `src/session` now stores runtime transcript history and TUI restores the latest session from `./rustcode/sessions/`
- Phase 3 completed: project-local/global settings now merge by source, `./rustcode/state/permission-events.json` records recent denies, and `always` writes only project-local permission rules
- Phase 3 completed: pending approvals now persist into session JSON and restore across process restarts as inline approval cards
- Phase 3 completed: TUI now has minimal Claude Code-style local control-plane commands for `/resume` and `/permissions`
- Phase 4 completed: added a shared `src/input` pipeline and moved TUI / REPL / `query` local command parsing onto unified slash command handling
- Phase 4 completed: added baseline `/help` `/clear` `/compact` `/permissions` `/model` `/status` `/resume` command support across the main entrypoints
- Phase 4 completed: added `src/compact` with manual `/compact`, baseline auto-compact after completed turns, compact settings, and transcript compact-boundary persistence
- Phase 3 completed: permission rules now support broader wildcard matching, and session metadata search now includes latest user summaries plus `kind:` filtering for `/resume`
- Phase 4 completed: compact now supports reactive pre-turn compaction, micro-compaction, reserved completion budget, and pending-message-aware budget decisions
- Verification completed: `cargo fmt`, `cargo check`
- Verification blocked: targeted `cargo test -q ...` invocations for new unit tests still fail in this Windows environment with `os error 5` when invoking `rustc` for test compilation
- Verification note: full `cargo test -q` reaches execution here, but the suite still contains an unrelated failing i18n assertion in `i18n::translator::tests::test_translate`
- Phase 5 in progress: TUI now restores chat-wheel scrolling after copy-mode changes, surfaces runtime tool progress rows while a turn is executing, and streams OpenAI-style assistant text into the transcript incrementally instead of waiting for the full turn to finish
- Phase 5 in progress: provider streaming now handles Anthropic SSE text/thinking/tool-use events and OpenAI-style streamed tool_call deltas, allowing token-level transcript updates even on tool-enabled turns when the upstream provider supports streaming
- Phase 5 in progress: assistant transcript rendering now uses a Claude-style markdown pipeline in the TUI, including headings, lists, blockquotes, code blocks, and responsive table fallback
- Phase 5 in progress: transcript chat lines now preserve styled spans for rendering while keeping plain-text copies for selection/copy behavior
- Phase 5 completed: approval cards now include bounded tool context, copy/selection status is surfaced in the sticky prompt area, scrollback respects manual navigation vs auto-follow, and live tool progress reuses a single preparing -> result row
- Phase 6 in progress: task-oriented subagent runtime landed with `task_create` / `task_list` / `task_get` / `task_update`, file-backed task persistence, isolated child sessions, and TUI task completion notifications
- Phase 6 in progress: agent definitions now carry per-agent context/memory/permission profiles, child agents load optional memory plus parent context summaries, and direct `run_agent` uses the same runtime contract as background subagents
- Phase 6 in progress: TUI now polls and displays queued/running child-agent progress for the active parent session instead of only surfacing completion/failure notifications after the fact
- Phase 6 in progress: child-agent permission requests now bubble back into the parent TUI, task-bound approvals resume the isolated child session instead of failing, and background subagents can pause on approval without losing state
- Phase 6 in progress: agent tool execution profiles now extend beyond the builtin allowlist model, with per-agent toggles for task tools, builtin allowlists, MCP tools, and external MCP tools
- Phase 6 in progress: subagent execution now honors `max_turns` through a bounded continuation loop, allowing limited multi-turn completion when a turn ends empty after tool use or is truncated by the model
- Phase 6 completed: child-task lineage now flows through contracts, task detail output, and parent TUI progress ordering, and bounded continuation behavior is locked by dedicated unit tests
- Phase 7 in progress: session persistence now carries stable message ids and fork metadata, enabling `/branch` plus message-bound rewind semantics instead of only linear `/resume`
- Phase 7 in progress: builtin `file_write` / `file_edit` now attach file-history metadata into tool results, and the TUI can rewind tracked files or the active conversation to a prior user message
- Phase 7 in progress: TUI branch/rewind flows now support Claude-style no-arg message pickers, rewind confirmation previews, and unique-prefix message id resolution
- Phase 7 in progress: `execute_command` now participates in rewind via bounded project snapshots and batch file-history metadata
- Phase 7 completed: transcript user turns now surface stable short message ids, child/fork session restore keeps lineage visible, and rewind previews distinguish tracked command/file origins including truncation warnings
- Phase 8 in progress: removed the old `gui-egui` default path from the root crate and introduced a dedicated `src-tauri/` desktop host crate plus `gui/` React/Vite frontend workspace
- Phase 8 in progress: the desktop bridge now exposes bootstrap/settings/session/submit/approval commands on top of `QueryEngine`, and forwards runtime progress as Tauri events for thinking/text/tool/approval updates
- Phase 8 in progress: initial desktop UI now supports onboarding gating, settings editing, session restore, transcript rendering, prompt submission, and approval resume flows
- Phase 8 verification completed: `cargo fmt`, `cargo check`, `cargo check -p rustcode-tauri`, `pnpm install`, `pnpm run build`
