# Phase 03 Permissions And Session

## Goal

将权限判断和 transcript/session 持久化引入主循环，并向 `claude-code-rev-main` / `claude-code-main (2)` 的项目本地配置和审批流靠拢。

## Completed

- 新增 `src/permissions/mod.rs`
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
- 新增项目本地 `./rustcode/` 目录语义，对标 Claude Code 的 `.claude/`
- `always allow` / `always deny` 现在会把规则写入 `./rustcode/settings.local.json`
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
- `query` / REPL / agent service 遇到 `AwaitingApproval` 时会返回明确提示，而不是假装成功

## Remaining

- 更丰富的规则匹配能力
- 跨进程恢复“未决审批卡片”
- GUI 审批流
- 更完整的 session 命名/索引/搜索
- Claude Code 风格更细的审批界面和工具进度展示

## Risks / Blockers

- `Settings::load()` 当前会把项目本地权限/session 覆盖合并到运行时设置里，但 `save()` 仍然是全局保存路径，后续需要进一步把“全局保存”和“本地覆盖”完全分层
- 测试编译依旧被当前 Windows 环境的 `os error 5` 阻塞，无法完成 `cargo test`

## Next

继续补 Phase 3 剩余边角，并准备进入 Phase 4 的输入预处理 / slash commands。
