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

Status: pending

### Phase 3

- 权限系统
- transcript/session 持久化
- 可恢复会话

Status: pending

### Phase 4

- 输入预处理
- slash commands
- compact / token budget

Status: pending

### Phase 5

- Claude Code 风格 TUI 对齐
- transcript-first layout
- tool progress / permission dialog / copy mode

Status: pending

### Phase 6

- agent extensions
- 子 agent runtime
- 更细粒度的 agent-specific context

Status: pending

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
- Verification completed: `cargo fmt`, `cargo check`
- Verification blocked: `cargo test -q query_loop` and `cargo check --tests` failed in this Windows environment with `os error 5` when invoking `rustc` for test compilation
