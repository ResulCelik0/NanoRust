# NanoRust

A small Rust clone of the nano text editor.

## Build

    cargo build --release      # binary: target/release/nanorust
    cargo test

## Usage

    nanorust [OPTIONS] [+LINE] FILE

A missing FILE is opened as a new buffer. `+LINE` starts on that line.

### Word documents (.docx)

    nanorust report.docx

A `.docx` file opens as rich text: one line per paragraph, with bold, italic and
underline kept per character. A missing `.docx` is created on save. Saving
writes the document back as `.docx`; other package parts (styles, headers, ...)
are carried through unchanged.

| Key | Action |
|---|---|
| Alt+1 / Alt+2 / Alt+3 | toggle bold / italic / underline on the selection, or for text typed next |
| Ctrl+R | insert an image: prompts for a PNG or JPEG path |

Images appear in the text as a single placeholder character (shown as
`[image: name]`) that can be moved, cut, pasted and deleted like any char.

Limitations for `.docx`:

- Tables are flattened to text lines; cell and row structure is lost on save.
- A soft line break (`w:br`) becomes a new paragraph; page breaks are dropped.
- Paragraph and section properties are regenerated, not preserved; only
  bold, italic and underline run formatting is kept.
- Only inline PNG and JPEG images can be inserted (at most 6,400 per
  document); text boxes and other drawings are ignored.
- Alt+1/2/3 and Ctrl+R have no effect in plain-text files.

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
