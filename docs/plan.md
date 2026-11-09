# NanoRust — Plan

## Stage 1: Core editor engine and file I/O
Goal: a tested, UI-free editing core plus load/save.
- [ ] Cargo project `nanorust` (lib + bin), CI-free; `cargo fmt`/`clippy` clean (fmt/clippy not installed; builds, unverified lint)
- [x] `buffer`: insert/delete char, newline, backspace, cursor movement (char, word, line, page, file), UTF-8 safe
- [x] Selection, cut/copy/paste lines, cut-to-end
- [x] `undo` with redo
- [x] `fileio`: load, save, atomic write, CRLF/final-newline preservation, missing file = new buffer
- [x] `search`: forward/backward find, case toggle, replace one/all
Acceptance: `cargo test` passes with unit tests for each module, including multibyte text, undo/redo
round trips, and save→load identity on temp files.

## Stage 2: Terminal UI and a usable editor
Goal: `nanorust FILE` is an interactive editor with the basic nano workflow.
- [x] Terminal setup/teardown with panic-safe restore; resize handling
- [x] `keys` → `Action` mapping with nano bindings; arrow/Home/End/PgUp/PgDn/Del
- [x] Rendering: title bar, text with horizontal/vertical scrolling, tabs, wide chars, status line, shortcut bar
- [x] Prompts: write-out filename, exit with unsaved-changes confirm, modified flag
- [x] Args: `+LINE`, `--version`, `--help`
Acceptance: render tests against a virtual screen (scrolling, tabs, wide chars); action-sequence tests
for edit→save→exit flow; an end-to-end test running the binary in a pty-less mode or via scripted
events that edits and saves a file.

## Stage 3: Search, replace, goto, help and options
Goal: feature parity for everyday nano use.
- [x] Search/replace prompts (Ctrl+W, Ctrl+\, Alt+W repeat, case/backwards toggles) with highlight of match
- [x] Goto line[,col] (Ctrl+_), cursor position (Ctrl+C), mark/selection rendering (Ctrl+6)
- [x] Undo/redo keys (Alt+U / Alt+E), cut/paste keys (Ctrl+K/U)
- [x] Help screen (Ctrl+G), `-l`, `-T`, `-E` options
- [x] README with usage and key table
Acceptance: tests for prompt state machine and each option; scripted end-to-end scenario covering
search+replace+save; `cargo clippy -- -D warnings` and `cargo fmt --check` pass.
