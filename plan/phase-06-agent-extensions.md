# Phase 06 Agent Extensions

## Goal

把现有 `services::agents` 从 prompt wrapper 提升为真正的 agent runtime 扩展层。

## Completed

- 子 agent 的最小 task runtime 已落地：`task_create` / `task_list` / `task_get` / `task_update`
- task store 已持久化到项目 `rustcode/state/tasks/`
- child agent 现在会创建隔离 session，并在完成后把结果写回 task store
- TUI 会轮询当前 session 相关的 task 完成/失败通知，并追加到 transcript

## Remaining

- final Claude-style fork/replay polish if needed

## Risks / Blockers

- 需要基于统一 runtime 和 transcript 之上实现

## Next

继续补权限冒泡、更接近 Claude 的 fork/subagent replay，以及多轮 subagent execution profile。

## Latest Update

- Phase 6 slice 2 completed: added agent-specific memory loading from `~/.rustcode/agent-memory/` and project-local `rustcode/agent-memory*/`
- Phase 6 slice 2 completed: child agent execution now injects agent preamble, optional memory, parent compact/recent context summary, and a stricter task contract
- Phase 6 slice 2 completed: `services::agents::run_agent` and background child tasks now share a unified execution path through `agents_runtime::executor`
- Phase 6 slice 2 completed: background-safe permission normalization now converts interactive `ask` rules into non-interactive denial for agent runs, preventing stuck background subagents
- Phase 6 slice 3 completed: TUI now surfaces in-flight child agent progress for the active parent session, showing queued/running subagent summaries before completion notifications arrive
- Phase 6 slice 4 completed: child-agent permission requests now bubble into the parent TUI as approval cards, and approved child tasks resume from their isolated child sessions instead of failing immediately
- Phase 6 slice 4 completed: agent definitions now expose richer tool profiles, allowing per-agent control over builtin allowlists plus task tools, MCP tools, and external MCP tools
- Phase 6 slice 4 completed: subagent execution now honors `max_turns` with a bounded continuation loop for empty / truncated completions, enabling limited multi-turn completion beyond the previous single-turn model
