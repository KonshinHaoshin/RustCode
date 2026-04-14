# Runtime Model Mapping

## Source Of Truth

- Claude Code runtime reference: `claude-code-rev-main/src`
- Rust implementation target: `src/runtime`

## Mapping

### Query engine

- Claude Code: `QueryEngine.ts`, `query.ts`
- RustCode target: `runtime/query_engine.rs`, `runtime/query_loop.rs`

### Runtime message model

- Claude Code: block/message structures passed through query loop
- RustCode target: `runtime/types.rs`

### Tool orchestration

- Claude Code: `services/tools/toolOrchestration.ts`, `toolExecution.ts`
- RustCode target: future `tools_runtime/*`

### Permissions

- Claude Code: `useCanUseTool.tsx`, `utils/permissions/*`
- RustCode target: future `permissions/*`

### Session and transcript

- Claude Code: `utils/sessionStorage.ts`
- RustCode target: future `runtime/transcript.rs` and session integration

### Input processing

- Claude Code: `utils/processUserInput/*`
- RustCode target: future `slash/*` and input pipeline

### Compact

- Claude Code: `services/compact/compact.ts`
- RustCode target: future `compact/*`
