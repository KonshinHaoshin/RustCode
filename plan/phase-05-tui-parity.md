# Phase 05 TUI Parity

## Goal

将当前 TUI 升级为接近 Claude Code 的 transcript-first agent frontend。

## Completed

- 恢复聊天区滚轮滚动，不再被选择逻辑吞掉
- TUI worker 改为消费 runtime 进度事件，而不是仅等待最终 turn 结果
- 执行中可见 tool request / tool result 进度行
- OpenAI-style 纯文本回复会增量刷入 transcript，而不是完成后一口气展示
- Anthropic-style `text_delta` / `thinking_delta` / `tool_use` SSE 事件已接入
- OpenAI-style 带 `tool_calls` 的 streaming delta 已接入，可在工具回合中继续增量输出正文
- assistant transcript 现已按 markdown 渲染，支持标题、列表、引用、代码块、表格和行内样式
- TUI transcript 行模型已升级为富文本 spans，同时保持复制选择依赖的 plain text 映射
- markdown 渲染加入了内容缓存和窄终端表格自动降级

## Remaining

- permission dialogs 继续细化
- sticky prompt equivalent
- copy mode 继续细化
- refined scrollback behavior
- 更细的 tool-call 构建中间态展示仍可继续打磨

## Risks / Blockers

- 需要依赖前几个 phase 的 runtime 事件模型

## Next

继续补齐更接近 Claude Code 的 transcript/front-end 细节，重点是更完整的流式 markdown 增量优化、权限卡片细化和滚动行为打磨。
