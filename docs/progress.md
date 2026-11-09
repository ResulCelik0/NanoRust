# Progress

## Stage 1: Core editor engine and file I/O
Built the UI-free core: Cargo project (lib + thin bin), `buffer` (cursor, movement, selection, cut/copy/paste, `Edit` with inverse), `undo` (History with redo), `search` (find/replace one/all, case toggle, backward, wrap), `fileio` (load, atomic save, LF/CRLF and final-newline preservation).
Tested with 37 unit tests, including multibyte text, undo/redo round trips and save→load identity on temp files.
Not done: rustfmt/clippy are not installed in this environment, so lint was never run; the first plan item stays unticked until it is.

## Stage 2: Terminal UI and a usable editor
Built `args` (`+LINE`, `--help`, `--version`, `-l/-T/-E` parsed), `term` (raw mode/alt screen guard, panic hook), `keys` (nano bindings → `Action`), `editor` (state, `handle(Action)`, write-out and save-on-exit prompts, modified flag, undo/redo), `ui` (pure `render` to a virtual `Screen`, scrolling by display column, tabs, wide chars, diff `draw`) and a thin `main` event loop.
Tested with 72 unit tests (key mapping, action-sequence edit→save→exit flows, render tests incl. tiny sizes) and 5 end-to-end tests (`tests/e2e.rs`) running the binary under a pty via `script`.
Not done: fmt/clippy not installed, so lint is unverified; tab width fixed at 8 (`-T` wiring is stage 3); interactive use was only exercised through the scripted tests.

## Stage 3: Search, replace, goto, help and options
Built the prompt state machine (search/replace with case and backwards toggles, per-match Y/N/A confirm, Alt+W repeat, goto `line[,col]`, cursor position, mark), help screen, `-l`/`-T`/`-E` (line-number gutter, tab width, tabs-to-spaces), selection and match highlighting, and a README with key table.
Tested with 97 unit tests (prompt transitions, options, render of gutter/tabs/selection/help) and 8 end-to-end pty tests, including search+replace+goto+cut/paste+undo+save.
Not done: rustfmt/clippy are not installed, so lint is unverified; no test drives a real search through to the rendered highlight.

## Stage 4: DOCX model, reader and writer (request #1)
Added `zip` and `quick-xml`; `rich` (Style flags, per-line style vectors aligned via `apply(&Edit)`, `RichEdit` records for exact undo/redo incl. restored styles, range toggle, image table with U+E000+n placeholders); `docx` (`read`/`write` of text, tabs, breaks, b/i/u, inline images, verbatim passthrough of unknown parts plus package rels/content types); `fileio` `.docx` load/save (zip magic check, atomic write, corrupt input errors without overwriting).
Tested with 114 unit tests (style alignment under edits/undo, round trips, images, passthrough, corrupt/truncated input, multibyte) plus the existing e2e tests.
Not done: not yet wired into `Editor`/UI (stage 5); rustfmt/clippy not installed, so lint is unverified.

## Stage 5: Editor and UI integration, images (request #1)
`Editor` routes `.docx` through `fileio` into buffer + rich layer; Alt+1/2/3 toggle bold/italic/underline (selection or typing state, undoable); Ctrl+R inserts a PNG/JPEG shown as `[image: name]`; UI renders styles via SGR attrs, shortcut bar/help/README updated.
Tested with 123 unit tests (action sequences open→style→image→save→reload, undo alignment, render attrs/placeholder, keys) and 9 pty e2e tests, including docx edit + PNG insert + save + re-read.
Not done: rustfmt/clippy not installed, so lint is unverified.
