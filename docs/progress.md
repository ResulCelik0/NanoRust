# Progress

## Stage 1: Core editor engine and file I/O
Built the UI-free core: Cargo project (lib + thin bin), `buffer` (cursor, movement, selection, cut/copy/paste, `Edit` with inverse), `undo` (History with redo), `search` (find/replace one/all, case toggle, backward, wrap), `fileio` (load, atomic save, LF/CRLF and final-newline preservation).
Tested with 37 unit tests, including multibyte text, undo/redo round trips and save→load identity on temp files.
Not done: rustfmt/clippy are not installed in this environment, so lint was never run; the first plan item stays unticked until it is.
