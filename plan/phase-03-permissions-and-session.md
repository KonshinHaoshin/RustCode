# Phase 03 Permissions And Session

## Goal

将权限判断和 transcript/session 持久化引入主循环，并向 `claude-code-rev-main` / `claude-code-main (2)` 的项目本地配置和审批流靠拢。

## Completed

- 新增 `src/permissions/mod.rs`
- 新增 `src/permissions/events.rs`
- 新增权限配置模型：
  - `PermissionMode`
  - `PermissionsSettings`
- `Settings` 中新增 `permissions`
- `Settings` 中新增 `session`
- `config set` 现在支持：
  - `permissions.mode`
  - `permissions.allow_tools`
  - `permissions.deny_tools`
  - `permissions.ask_tools`
  - `session.auto_restore_last_session`
  - `session.persist_transcript`
- 配置读取现在会合并项目本地 `./rustcode/settings.local.json`
- 项目本地与全局配置现在按字段做 source-aware merge，而不是整块覆盖
- 新增项目本地 `./rustcode/` 目录语义，对标 Claude Code 的 `.claude/`
- 新增 `./rustcode/state/permission-events.json`，记录最近的 deny / ask-deny 事件
- `always allow` / `always deny` 现在会把规则写入 `./rustcode/settings.local.json`
- `always allow` / `always deny` 现在只写 project-local 规则，不再把合并后的全局权限整包写回本地
- runtime 工具执行前现在会经过 `PermissionGate`
- `ask` 不再直接变成拒绝型 tool result，而是会返回 `AwaitingApproval`
- `QueryEngine` 已支持 `resume_after_approval(...)`
- TUI 已接入 inline approval：
  - `Allow Once`
  - `Deny Once`
  - `Always Allow`
  - `Always Deny`
- `src/session/mod.rs` 已升级为 runtime transcript store，持久化 `RuntimeMessage` / tool call / tool result
- TUI 会把 session 保存到项目本地 `./rustcode/sessions/`
- TUI 启动时会恢复最近 transcript session
- session schema 现在包含：
  - `status`
  - `pending_approval`
  - `project_root`
  - `entry_type`
- TUI 现在可以跨进程恢复“未决审批卡片”
- TUI 现在支持最小本地命令：
  - `/resume`
  - `/resume <session-id>`
  - `/permissions`
- `/permissions` 现在可以查看 global/local 规则来源，删除 local rule，并把 recent event 提升为 allow/deny/ask
- `query` / REPL / agent service 遇到 `AwaitingApproval` 时会返回明确提示，而不是假装成功

- permission matching now supports wildcard patterns beyond simple prefix rules, while keeping deny > ask > allow precedence
- primary sessions can now auto-refresh placeholder names from the first user turn and expose latest user summaries for search/picker previews
- `/resume <query>` now supports metadata search plus `kind:` filtering for primary/forked/child sessions
## Remaining

- 更丰富的规则匹配能力
- GUI 审批流
- 更完整的 session 命名/索引/搜索
- Claude Code 风格更细的审批界面和工具进度展示

## Risks / Blockers

- `Settings::save()` 仍然是全局保存路径；虽然项目本地权限现在有单独 helper，但更彻底的 local/global 分层仍值得继续收敛
- 这台 Windows 环境里针对 test target 的 `rustc` 调用仍然频繁触发 `os error 5`，导致新增单元测试无法稳定执行
- 仓库现有完整测试套件里还存在一个与本次改动无关的 i18n 断言失败：`i18n::translator::tests::test_translate`

## Next

进入 Phase 4，开始实现 Claude Code 风格的输入预处理、本地 slash commands 扩展和 compact/token-budget 基础设施。
