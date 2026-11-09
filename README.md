# NanoRust

A small Rust clone of the nano text editor.

## Build

    cargo build --release      # binary: target/release/nanorust
    cargo test

## Usage

    nanorust [OPTIONS] [+LINE] FILE

A missing FILE is opened as a new buffer. `+LINE` starts on that line.

## Options

| Option | Meaning |
|---|---|
| `-l`, `--linenumbers` | show line numbers |
| `-T N`, `--tabsize=N` | tab width |
| `-E`, `--tabstospaces` | insert spaces when Tab is pressed |
| `-h`, `--help` | usage |
| `-V`, `--version` | version |

## Keys

| Key | Action |
|---|---|
| Ctrl+O | write out (save) |
| Ctrl+X | exit (asks to save if modified) |
| Ctrl+W | search; Alt+C case toggle, Alt+B backwards toggle |
| Alt+W | repeat last search |
| Ctrl+\ | search and replace (Y / N / A for all / Esc) |
| Ctrl+_ | go to `line[,column]` |
| Ctrl+C | show cursor position (cancels a prompt) |
| Ctrl+6 / Alt+A | set/unset mark |
| Ctrl+K / Ctrl+U | cut line or selection / paste |
| Alt+U / Alt+E | undo / redo |
| Ctrl+G | help screen |
| Ctrl+A / Ctrl+E | start / end of line |
| Ctrl+Y / Ctrl+V | page up / down |
| Arrows, Home, End, PgUp, PgDn, Del | movement and deletion |

## Limitations

- Single buffer; no multi-file editing, no syntax highlighting.
- Search is plain text (no regular expressions).
- Only the keys in the table are bound.
- Tested on Unix terminals only.
