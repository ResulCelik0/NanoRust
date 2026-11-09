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
