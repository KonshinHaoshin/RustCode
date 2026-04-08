# Phase 08 Tauri Desktop Migration

## Goal

Replace the old egui GUI path with a Tauri desktop shell backed by a React/Vite frontend while reusing the existing Rust runtime, session, approval, and streaming infrastructure.

## Completed

- Root crate defaults no longer depend on `gui-egui`, and the legacy `rustcode-gui` bin is removed from the default build path.
- Added `src-tauri/` as a dedicated desktop host crate wired to the root `rustcode` library.
- Added `gui/` as a React 18 + Vite + TypeScript frontend workspace.
- Desktop bridge commands landed for:
  - `bootstrap_gui_state`
  - `load_settings`
  - `save_settings`
  - `complete_onboarding`
  - `list_sessions`
  - `restore_session`
  - `submit_prompt`
  - `respond_to_approval`
- Runtime progress is forwarded to the desktop frontend through Tauri events for:
  - `turn_started`
  - `model_request`
  - `thinking_text_chunk`
  - `assistant_text_chunk`
  - `tool_call`
  - `tool_result`
  - `awaiting_approval`
  - `turn_completed`
- Initial desktop UI supports onboarding gating, settings editing, session restore, transcript display, prompt submission, and approval resume.
- Added a placeholder Windows icon so `tauri-build` can generate the Windows resource file.
- Frontend package scripts now target `../src-tauri/tauri.conf.json`, matching the repo layout instead of assuming `src-tauri/` lives under `gui/`.
- Desktop bridge now includes:
  - `create_session`
  - `delete_session`
  - `list_user_turn_targets`
  - `preview_rewind`
  - `rewind_session`
  - `branch_session`
  - `list_active_tasks`
  - `open_project_folder`
  - `choose_working_directory`
- The React/Tauri shell has been redesigned to a minimal light desktop UI with:
  - left rail navigation
  - session list with delete affordance
  - home / thread / settings / automation views
  - floating bottom composer
  - structured markdown rendering for assistant messages
  - inline approval and rewind preview cards
- Desktop workspace handling is no longer fixed to the repo root; the UI can now select a workspace directory and switch subsequent session, task, file-history, and chat context to that directory.
- Window defaults were reduced to a smaller initial size for a less oversized first launch.
- The desktop dev loop now mitigates common Windows restart friction by:
  - renaming the Tauri library target to avoid bin/lib PDB filename collisions
  - routing `gui` `tauri:dev` through a small PowerShell wrapper that stops stale `rustcode-tauri` processes before relaunch

## In Progress

- Improve the naming and polish of desktop actions so workspace selection, reveal-in-folder, rewind/restore, and task surfaces read clearly without internal terminology leaking into the UI.
- Keep an eye on `tauri dev` on Windows for any remaining binary-lock edge cases beyond the current stale-process cleanup script.

## Remaining

- Decide whether to keep separate `Reveal` / `Select workspace` actions or collapse them into one tighter workspace picker flow.
- Align desktop transcript behavior with TUI compact/session metadata, including fork lineage and compact boundaries.
- Decide when to fully remove or archive the old `src/gui/` code.
- Update public docs and install guidance so Tauri becomes the official GUI story.

## Risks / Blockers

- The current environment can build both Rust and frontend artifacts, but repeat `tauri dev` smoke passes on Windows can fail if a previous `rustcode-tauri.exe` is still locking the binary.
- Existing repo worktree already contains large unrelated in-flight changes; desktop migration work must avoid overwriting those integrations.
- Desktop DTOs and event names are now frontend contracts, so future changes need append-only evolution to avoid breaking the UI.

## Verification

- `cargo fmt`
- `cargo check`
- `cargo check -p rustcode-tauri`
- `pnpm install` in `gui/`
- `pnpm run build` in `gui/`
- `pnpm run tauri:dev` smoke pass reached desktop launch, with one follow-up Windows file-lock failure during hot reload when an old process remained alive.

## Exit Criteria

- `tauri dev` launches successfully and the desktop shell completes onboarding/settings/session restore/submit/approval flows end-to-end.
- Transcript rendering is no longer placeholder-level and supports structured assistant output.
- Desktop session controls support restore plus branch/rewind basics.
- Tauri is documented as the primary desktop path and the old egui GUI is no longer part of the supported default flow.
