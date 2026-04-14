# Phase 01 Runtime Foundation

## Goal

建立最小可运行的 runtime 入口，让 TUI、REPL、`query`、`run_agent` 不再直接调用 `ApiClient.chat`。

## Completed

- 建立 `plan/` 文档骨架
- 调整 `.gitignore`，允许跟踪 `plan/*.md`
- 新增 `src/runtime/mod.rs`
- 新增最小运行时消息模型：`RuntimeRole`、`RuntimeMessage`
- 新增最小查询循环：`QueryLoop`、`ModelGateway`
- 新增 `QueryEngine`
- TUI 改为经由 `QueryEngine` 发起纯文本 turn
- REPL 改为经由 `QueryEngine` 发起纯文本 turn
- CLI `query` 改为经由 `QueryEngine`
- `services::agents::run_agent` 改为经由 `QueryEngine`
- 新增 runtime-focused tests in `src/runtime/query_loop.rs`
- 已跑 `cargo fmt`
- 已跑 `cargo check`

## Remaining

- Phase 1 scope 内无剩余代码项
- 下一个阶段需要开始设计 canonical tool model 和 tool runtime

## Risks / Blockers

- TUI 当前线程模型是手工 thread + channel，需要保持最小改动
- 当前 `api::ChatMessage` 仍是 transport 类型，runtime 需要先做薄映射，避免大范围重构
- `cargo test -q query_loop` 在当前 Windows 环境里因为 `os error 5` 未能完成，原因看起来是测试编译阶段调用 `rustc` 被拒绝访问，不是普通编译错误
- `cargo check --tests` 同样被相同的 `os error 5` 阻塞

## Next

进入 Phase 2，开始工具模型和工具执行编排设计。
