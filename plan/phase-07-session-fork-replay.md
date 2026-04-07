# Phase 07 Session Fork Replay

## Goal

Extend session persistence from a simple linear resume model into a fork / rewind / replay model that is closer to Claude Code.

## Completed

- Session persistence now carries stable message ids and parent links.
- Session metadata now supports `forked` sessions with `forked_from_session_id` and `forked_from_message_id`.
- `SessionManager` can create forked sessions from the current session or a specific user message.
- Slash commands now include `/branch`, `/fork`, `/rewind`, and `/rewind-files`.
- The TUI supports branch, full rewind, and files-only rewind flows for the active session.
- Builtin `file_write` and `file_edit` attach file-history metadata into tool results.
- `src/file_history` can restore tracked files back to a selected user turn.
- Resume lists now show session kind, and forked sessions can be restored via `/resume`.
- The TUI now supports picker-based no-arg flows for `/branch`, `/rewind`, and `/rewind-files`.
- Rewind selection now previews tracked file changes before confirming a conversation rewind.
- `execute_command` now snapshots the project tree and records batch file-history metadata for rewind.
- Forked sessions now include a lineage system notice so restored state keeps branch provenance visible.

## Remaining

- Command tracking intentionally remains a bounded project snapshot approach rather than a journal-based tracker.
- Child-task replay is lineage-aware on restore, but not yet a full transcript subtree replay model.

## Risks / Blockers

- Command file tracking is capped to a bounded number of files per execution to avoid runaway snapshot cost.
- Windows test/build execution in this environment can still require elevated runs due intermittent `os error 5` access issues.

## Verification

- `cargo fmt`
- `cargo check`
- `cargo test -q parses_basic_slash_commands`
- `cargo test -q command_snapshot_detects_modified_and_new_files`
- `cargo test -q capture_and_rewind_new_file_restores_absence`
- `cargo test -q create_fork_session_clones_history_with_new_ids`
