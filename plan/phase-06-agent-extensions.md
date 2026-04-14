# Phase 06 Agent Extensions

## Goal

Elevate `services::agents` from a prompt wrapper into a real agent-runtime extension layer with child tasks, isolated sessions, and agent-specific execution profiles.

## Completed

- Landed task-oriented subagent runtime primitives: `task_create`, `task_list`, `task_get`, and `task_update`.
- Persisted task state under project-local `rustcode/state/tasks/` storage.
- Spawned child agents into isolated child sessions and wrote results back into the task store.
- Surfaced task completion and failure notifications back into the parent TUI transcript.
- Added agent-specific memory loading from `~/.rustcode/agent-memory/` and project-local `rustcode/agent-memory*/` sources.
- Injected agent preamble, optional memory, parent compact/recent context summary, and stricter task contracts into child execution history.
- Unified direct `run_agent` and background child-task execution through `agents_runtime::executor`.
- Normalized background-safe permission behavior so interactive `ask` rules become non-interactive denial for background subagents.
- Surfaced in-flight child-agent progress in the parent TUI, including queued, running, and awaiting-approval states.
- Bubbled child-agent permission requests back into the parent TUI as approval cards and resumed approved child tasks from their isolated child sessions.
- Expanded agent tool execution profiles with per-agent builtin allowlists, task tools, MCP tools, and external MCP tools.
- Added bounded `max_turns` continuation behavior for empty or truncated completions.
- Added lineage polish so child contracts and `task_get` expose parent session, child session, pending approval, and max-turn context.
- Added targeted tests that lock continuation behavior and lineage/task formatting.

## Remaining

- None for the current Phase 6 scope.

## Risks / Blockers

- Full transcript subtree replay remains out of scope; current replay/restore behavior is lineage-aware rather than a new replay model.
- Windows test execution in this environment can still intermittently hit `os error 5` and may require elevated targeted test runs.

## Verification

- `cargo check`
- targeted unit tests for child-task lineage, task detail formatting, and bounded continuation behavior

## Next

Phase 6 completed. Later work should move to subsequent phases unless new agent-runtime regressions reopen this area.
