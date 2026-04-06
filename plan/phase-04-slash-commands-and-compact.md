# Phase 04 Slash Commands And Compact

## Goal

实现 Claude Code 风格输入预处理、slash commands 和 compact。

## Completed

- 新增统一输入预处理层：
  - `src/input/mod.rs`
  - `src/input/pipeline.rs`
  - `src/input/slash.rs`
  - `src/input/types.rs`
- TUI / REPL / `query` 现在都会先经过统一 slash/input pipeline，而不是各自零散解析
- 新增内建 slash commands：
  - `/help`
  - `/clear`
  - `/compact [instructions]`
  - `/permissions`
  - `/model [name]`
  - `/status`
  - `/resume`
- TUI 不再用私有 `parse_local_command()`；已有 `/permissions` 和 `/resume` 已迁入统一命令语义
- REPL 现在支持 slash commands，同时保留旧 `help` / `status` / `reset` / `history` / `config` 兼容入口
- `query` 现在支持基础本地 slash commands 输出；需要交互 UI 的命令会明确报错，而不是静默忽略
- 新增 `src/compact/mod.rs`
- 新增 `src/compact/prompt.rs`
- 新增 `src/compact/service.rs`
- 新增 `Settings.compact`：
  - `enabled`
  - `auto_compact`
  - `max_turns_before_compact`
  - `max_tokens_before_compact`
  - `preserve_recent_turns`
  - `summary_model`
- `config set` 现在支持：
  - `compact.enabled`
  - `compact.auto_compact`
  - `compact.max_turns_before_compact`
  - `compact.max_tokens_before_compact`
  - `compact.preserve_recent_turns`
  - `compact.summary_model`
- `QueryEngine` 现在会在 turn 完成后执行基础 auto-compact 判定
- 现在支持手动 `/compact`
- compact 结果会把较早 transcript 压缩成一条 system summary，并保留最近窗口
- `QueryTurnResult` 现在会暴露：
  - `was_compacted`
  - `compaction_summary`
- session transcript 现在会把 compact summary 标记为 `CompactBoundary`

## Remaining

- Claude Code 级更复杂 compaction（microcompact / reactive compact / session-memory compact）
- 更细粒度的 token 预算估算；当前 auto-compact 只依赖最近 usage 和消息数
- `/permissions` / `/resume` 的 REPL 交互式文本体验仍然较基础，完整 UI 对齐放到 phase 5
- 更完整的 compact 进度展示和 post-compact UX 对齐

## Risks / Blockers

- compact 仍依赖 transcript/runtime state 的稳定性；当前只实现单一摘要式压缩
- Windows 环境下 `cargo test` 仍然会在 test-target 编译阶段触发 `os error 5`

## Next

- 进入 Phase 5，开始做 Claude Code 风格 TUI 对齐、transcript-first 布局，以及更细的本地命令展示/审批交互。
