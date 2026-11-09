# NanoRust — Design

## Goals
A Rust reimplementation of the GNU nano terminal text editor: modeless, keyboard-driven, with a
shortcut help bar, familiar key bindings (Ctrl+O save, Ctrl+X exit, Ctrl+K cut, Ctrl+U paste,
Ctrl+W search, Ctrl+\ replace, Ctrl+G help, Ctrl+_ goto line). Binary name: `nanorust`.

## Scope (v1)
- Open/edit/save a file (`nanorust [+LINE] FILE`), new files, UTF-8 text, tabs, CRLF preserved.
- Title bar, edit area, status line, two-line shortcut bar, soft scrolling, terminal resize.
- Cut/paste lines, selection (mark, Ctrl+6), undo/redo, search, replace, goto line, help screen.
- Options: `--linenumbers (-l)`, `--tabsize (-T)`, `--tabstospaces (-E)`, `--version`, `--help`.
- Linux terminals (also works on macOS).

## Non-goals
Syntax highlighting, multiple buffers, spell check, rc-file parsing, mouse, soft-wrap, justify,
backups/locking, exact bug-for-bug nano compatibility.

## Architecture
Cargo workspace-free single crate (lib + thin bin) so the core is unit-testable without a TTY.
- `buffer`: text as `Vec<String>` lines; cursor, edit operations, selection, cut buffer. Pure logic.
- `undo`: operation-based undo/redo stack over buffer edits.
- `search`: find/replace over buffer (case toggle, forward/backward, wrap).
- `fileio`: load/save (atomic write via temp file + rename, line-ending detection).
- `keys`: maps terminal events to an `Action` enum (nano bindings).
- `ui`: renders a `Editor` state to a frame (`Vec<String>`-like screen model) and diffs it to the
  terminal; prompts (save name, search, goto) as a small modal state machine.
- `editor`: application state + `handle(Action)`; no I/O besides via `fileio`.
- `main`: arg parsing (hand-written), terminal setup/teardown, event loop.

## Key decisions
- **crossterm** for raw mode, input events and drawing: the only runtime dependency; avoids
  unsafe termios code and works with a resize event. `unicode-width` for display columns.
- **Line vector buffer** (not rope): files edited by nano are small; simple and fast enough.
- **Action-based core**: UI-independent state makes nearly everything testable with plain unit tests;
  rendering is tested against a virtual screen model; a few end-to-end tests drive the binary.
- **Hand-written arg parsing**: tiny option set, avoids a dependency.
- **Atomic save** to avoid data loss on crash.
- Restore the terminal on panic (hook) so the shell is never left in raw mode.

## Rich text documents (request #1)
`.docx` files open as rich documents: text and bold/italic/underline plus inline images
(shown as `[image: name]` placeholders). New modules: `rich` (per-char style layer kept aligned by
applying the same `Edit`s as the buffer; image table; images are one private-use char in the text)
and `docx` (read/write OOXML via `zip` + `quick-xml`; unknown package parts pass through verbatim).
Plain-text editing is unchanged. Keys: Alt+1/2/3 style, Ctrl+R insert image. Details:
docs/requests/1-zengin-metin-belgelerinide-a-mak-istiyor.md. Rich-text editing is no longer a
non-goal; RTF/ODT, tables, and image display remain out of scope.
