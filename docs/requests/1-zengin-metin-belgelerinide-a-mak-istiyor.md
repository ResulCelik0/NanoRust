# Request #1: rich text documents (zengin metin belgeleri)

User request (Turkish): open rich text documents in the editor, edit them, and insert images.

## Interpretation
"Rich text document" = **Word `.docx`** (the dominant format; also what "insert image" implies).
`nanorust report.docx` opens it, shows formatted text in the terminal, lets the user edit text,
toggle bold/italic/underline, and insert images; saving writes a valid `.docx`. Plain files behave
exactly as before. A terminal cannot draw pictures, so an image is shown as an inline placeholder
`[image: name.png]` that is edited/deleted like one character. RTF/ODT are out of scope (could be a
later request).

## Design decisions
- **New deps (approved by this request):** `zip` (default-features off, `deflate`) for the OOXML
  container and `quick-xml` for parsing/writing XML. Hand-rolling either is riskier. CLAUDE.md is
  updated accordingly.
- **Plain text stays the source of truth.** `Buffer` keeps `Vec<String>` lines; paragraph = line.
  Formatting lives in a separate `rich` layer: a per-line `Vec<Style>` parallel to chars
  (`Style` = bit flags BOLD/ITALIC/UNDERLINE). Every change already goes through `Edit`, so the
  layer applies the same `Edit` (and its inverse on undo) to keep styles aligned; no second undo stack.
  Style toggles are their own undoable `Edit` variant over a range.
- **Images** are one private-use char `U+E000+n` in the text, `n` indexing an image table
  (`name`, `bytes`, content type) in the document. Cursor movement, delete, cut/paste, undo work
  unchanged; pasted copies reference the same image. Render shows `[image: name]` (display-width
  aware); unknown ids are dropped on save.
- **`docx` module** (pure, no terminal): `read(&[u8]) -> Document` and `write(&Document) -> Vec<u8>`.
  Reads `word/document.xml` paragraphs/runs (`w:t`, `w:tab`, `w:br`, `w:b`, `w:i`, `w:u`) and inline
  images (`w:drawing` → `r:embed` → `word/media/*`). Unsupported content (tables, headers, fields)
  is read as its text where possible; **unsupported parts of the package (styles, numbering,
  headers, unknown media) are carried through verbatim** on save so documents are not degraded.
  Write is atomic like `fileio`.
- **Format selected by extension** (`.docx`) and confirmed by the zip magic; a corrupt file gives a
  status-line error and does not open/overwrite anything. New `x.docx` that doesn't exist = empty
  rich document.
- **Keys:** nano's Ctrl+B/F/T are taken, so in edit mode: Alt+1 bold, Alt+2 italic, Alt+3
  underline (toggle on the selection, or for subsequently typed text when nothing is selected);
  Ctrl+R (nano's "insert file") inserts an image: prompts for a png/jpeg/gif path, rich documents only.
- Rendering uses crossterm attributes (bold/italic/underline) on the virtual `Screen` cells.

## Stage 4: DOCX model, reader and writer (no UI)
Goal: a tested `docx` + `rich` core.
- [ ] Add `zip`, `quick-xml`; update CLAUDE.md dependency rule
- [ ] `rich`: `Style`, per-line style vectors, `apply(Edit)` keeping them aligned, range style toggle, image table + placeholder chars
- [ ] `docx::read` / `docx::write` with text, tabs, breaks, bold/italic/underline, inline images, verbatim passthrough of other parts
- [ ] `fileio`: load/save `.docx` (atomic), error on corrupt input
Acceptance: unit tests: style alignment after insert/delete/newline/undo/redo; read of a fixture
docx built in the test (zip created in code) with styles and an image; write→read round trip
identity; passthrough of an unknown part; corrupt zip and missing `document.xml` give errors;
multibyte text; plain-text tests still pass.

## Stage 5: Editor and UI integration, images
Goal: `nanorust file.docx` is usable end to end.
- [ ] `Editor` holds an optional rich layer; open/save route by format; modified flag/undo cover style edits
- [ ] Keys/actions: Alt+1/2/3 style toggles, Ctrl+R insert image prompt (validates file and type, status errors)
- [ ] `ui`: styled cells, `[image: name]` placeholder, title bar shows `(docx)`; help screen and shortcut bar list new keys
- [ ] README section on rich documents; design.md kept in sync
- [ ] `.workploy/ci.yml` runs build, tests (including the e2e docx scenario)
Acceptance: action-sequence tests (type, bold selection, insert image, undo, save, reload and
compare); render tests for bold/italic/underline attributes and the image placeholder; e2e pty
test that opens a fixture docx, edits, inserts a PNG, saves, and re-reads the result with
`docx::read`; plain-text e2e tests unchanged; fmt/clippy run if available.
