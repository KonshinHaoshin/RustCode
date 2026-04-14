# Phase 02 Tool Runtime

## Goal

加入真正的工具调用协议、工具执行编排和 tool result 回填。

## Completed

- 新增 `src/tools_runtime/mod.rs`
- 新增 `ToolDefinition`
- 新增 builtin `ToolExecutor`
- 新增 `CompositeToolExecutor`
- 新增 `McpToolExecutor`
- 将 runtime message 模型扩展为：
  - `RuntimeToolCall`
  - `RuntimeToolResult`
  - `RuntimeRole::Tool`
- QueryLoop 现在支持：
  - 发送工具定义给 gateway
  - 接收 assistant tool calls
  - 执行 builtin tools
  - 追加 tool result 消息
  - 继续请求直到拿到最终 assistant 文本
- `ApiClient` 新增 `chat_with_tools`
- OpenAI/custom-open_ai 请求现在会发送 `tools` 和 `tool_choice: auto`
- OpenAI 响应现在兼容：
  - `content: null/empty`
  - `tool_calls`
  - tool result message fields
- Windows 上的 `execute_command` builtin 已改为 `cmd /C`
- runtime 默认工具执行器现在组合了：
  - builtin tools
  - namespaced MCP registry tools，例如 `mcp__file_read`
- 新增协议能力感知：
  - `ApiProtocol::supports_tool_calling`
  - `ResolvedApiTarget::supports_tool_calling`
- 当一个 turn 带工具定义时，请求目标会优先选择支持 tool calling 的 target
- 对不支持工具调用的 target，会自动发送空工具列表，避免构造无效工具请求
- 已补上 Anthropic 原生工具协议的基础支持：
  - 请求侧 `tools`
  - assistant `tool_use` 解析
  - tool result 消息映射
  - runtime <-> API 消息桥接
- 已补上外部 MCP server 的基础 stdio 动态工具接入：
  - `initialize`
  - `tools/list`
  - `tools/call`
  - 运行时命名空间工具名：`mcp_ext__{server}__{tool}`
- 对工具未找到、权限拒绝、执行失败等情况，runtime 会回填错误型 tool result

## Remaining

- 当前 phase 范围内无阻塞性剩余代码项
- 仍未完成的测试执行受当前 Windows `os error 5` 环境问题阻塞

## Risks / Blockers

- 现有 MCP 和 builtin tools 仍是两套注册机制，后续要收敛
- 当前外部 MCP 接入是基础 stdio client，尚未覆盖更复杂的长连接、通知流和持久会话
- 当前 Anthropic 支持是基础版协议映射，还没有覆盖更细的 stop reason、streaming tool events 和权限交互
- `cargo test -q query_loop` 在当前 Windows 环境仍然被 `os error 5` 阻塞

## Next

进入 Phase 3，继续把 interactive approval、transcript 和 session 持久化补齐。
