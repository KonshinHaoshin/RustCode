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

## In Progress

- Replace the current plain transcript rendering with richer Claude-style structured markdown rendering.
- Expand the desktop session control surface beyond restore/continue into branch, rewind preview, and rewind execution.
- Improve tool and approval UX so intermediate tool states are represented as first-class cards instead of generic activity rows.
- Run the desktop shell through full `tauri dev` smoke testing and fix any runtime integration gaps.

## Remaining

- Add desktop commands for branch / rewind / task inspection.
- Surface `turn_failed` and related error events so interrupted turns do not silently collapse into a generic error banner.
- Align desktop transcript behavior with TUI compact/session metadata, including fork lineage and compact boundaries.
- Decide when to fully remove or archive the old `src/gui/` code.
- Update public docs and install guidance so Tauri becomes the official GUI story.

## Risks / Blockers

- The current environment can build both Rust and frontend artifacts, but a real `tauri dev` GUI smoke pass still depends on local desktop/runtime availability.
- Existing repo worktree already contains large unrelated in-flight changes; desktop migration work must avoid overwriting those integrations.
- Desktop DTOs and event names are now frontend contracts, so future changes need append-only evolution to avoid breaking the UI.

## Verification

- `cargo fmt`
- `cargo check`
- `cargo check -p rustcode-tauri`
- `pnpm install` in `gui/`
- `pnpm run build` in `gui/`

## Exit Criteria

- `tauri dev` launches successfully and the desktop shell completes onboarding/settings/session restore/submit/approval flows end-to-end.
- Transcript rendering is no longer placeholder-level and supports structured assistant output.
- Desktop session controls support restore plus branch/rewind basics.
- Tauri is documented as the primary desktop path and the old egui GUI is no longer part of the supported default flow.
