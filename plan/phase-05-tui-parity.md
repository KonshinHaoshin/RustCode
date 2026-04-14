# Phase 05 TUI Parity

## Goal

Bring the TUI closer to a Claude Code style transcript-first frontend on top of the unified runtime.

## Completed

- Restored mouse-wheel transcript scrolling after earlier copy-mode regressions.
- Switched the TUI worker to consume runtime progress events instead of waiting only for final turn completion.
- Added streaming assistant text updates for OpenAI-style responses so transcript text appears incrementally.
- Added Anthropic SSE support for text, thinking, and tool-use streaming events.
- Added OpenAI-style streamed tool-call delta handling so tool-enabled turns can keep rendering incrementally.
- Upgraded assistant transcript rendering to a markdown-aware pipeline with headings, lists, quotes, code blocks, and table fallback.
- Preserved styled spans for transcript rendering while keeping plain-text mapping for selection and copy behavior.
- Added markdown content caching and narrow-terminal table degradation.
- Refined approval cards with origin-aware context, bounded argument previews, and tool risk/summary metadata.
- Refined transcript scrollback so manual scroll and auto-follow are distinct, oversized offsets are clamped, and streaming updates respect the user's current position.
- Refined copy/selection behavior so `Ctrl+C` copies the active selection, copy status is surfaced in the sticky prompt area, and selection clearing behaves predictably.
- Refined live tool progress so a single in-flight tool row transitions from preparing to final result instead of duplicating transient rows.

## Remaining

- None for the current Phase 5 scope.

## Risks / Blockers

- The TUI behavior depends on the runtime event model stabilized in earlier phases.
- Windows test execution in this environment can still intermittently hit `os error 5` and may require elevated targeted test runs.

## Verification

- `cargo check`
- targeted unit tests for selection, scrollback, approval labeling, and tool-progress helpers

## Next

Phase 5 completed. Continue with later phases unless new TUI regressions reopen this scope.
