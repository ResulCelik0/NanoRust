//! Editor state: buffer, history, file info, prompts, status and scrolling.
//!
//! The UI feeds [`Action`]s to [`Editor::handle`] and renders from the read
//! accessors. Nothing here touches the terminal.

use std::io;
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, Edit};
use crate::fileio::{self, LineEnding};
use crate::keys::Action;
use crate::undo::History;

/// Rows of terminal chrome around the text area (title bar, status line, two help lines).
pub const CHROME_ROWS: usize = 4;

/// Which prompt is currently open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    /// "File Name to Write:" with an editable filename.
    WriteOut,
    /// "Save modified buffer? (Y/N/Ctrl+C)" asked on exit with unsaved changes.
    SaveModified,
}

#[derive(Debug, Clone)]
struct Prompt {
    kind: PromptKind,
    text: String,
    /// Quit after a successful write (the exit flow).
    quit_after: bool,
}

/// The editor model.
#[derive(Debug)]
pub struct Editor {
    buffer: Buffer,
    history: History,
    path: Option<PathBuf>,
    line_ending: LineEnding,
    final_newline: bool,
    modified: bool,
    status: Option<String>,
    quit: bool,
    prompt: Option<Prompt>,
    /// First visible buffer line.
    pub top_line: usize,
    /// First visible character column.
    pub left_col: usize,
    /// Text area size `(columns, rows)`.
    viewport: (usize, usize),
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

impl Editor {
    /// An empty, unnamed editor with an 80x24 terminal assumed.
    pub fn new() -> Self {
        Editor {
            buffer: Buffer::new(),
            history: History::new(),
            path: None,
            line_ending: LineEnding::Lf,
            final_newline: true,
            modified: false,
            status: None,
            quit: false,
            prompt: None,
            top_line: 0,
            left_col: 0,
            viewport: (80, 24 - CHROME_ROWS),
        }
    }

    /// Opens `path` (a missing file gives an empty new buffer named `path`),
    /// optionally moving to the one-based `line`. Errors (e.g. invalid UTF-8)
    /// are returned for the caller to report.
    pub fn open(path: &Path, line: Option<usize>) -> io::Result<Editor> {
        let loaded = fileio::load(path)?;
        let mut ed = Editor::new();
        let n = loaded.lines.len();
        ed.buffer = Buffer::from_text(&loaded.lines.join("\n"));
        ed.line_ending = loaded.line_ending;
        ed.final_newline = loaded.final_newline || loaded.is_new;
        ed.path = Some(path.to_path_buf());
        if let Some(l) = line {
            ed.buffer.goto(l, None);
        }
        ed.status = Some(if loaded.is_new {
            "New File".to_string()
        } else {
            format!("Read {n} lines")
        });
        ed.scroll_to_cursor();
        Ok(ed)
    }

    // ---- read accessors ----

    /// The text buffer.
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }
    /// The open prompt, if any.
    pub fn prompt_kind(&self) -> Option<PromptKind> {
        self.prompt.as_ref().map(|p| p.kind)
    }
    /// Text typed into the open prompt (the filename for `WriteOut`).
    pub fn prompt_text(&self) -> Option<&str> {
        self.prompt.as_ref().map(|p| p.text.as_str())
    }
    /// Prompt label to display, if a prompt is open.
    pub fn prompt_label(&self) -> Option<&'static str> {
        self.prompt.as_ref().map(|p| match p.kind {
            PromptKind::WriteOut => "File Name to Write",
            PromptKind::SaveModified => "Save modified buffer? (Y/N/Ctrl+C)",
        })
    }
    /// Current status message, if any.
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }
    /// Whether the buffer has unsaved changes.
    pub fn modified(&self) -> bool {
        self.modified
    }
    /// File path, if named.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    /// Name for the title bar: the file name, or "New Buffer".
    pub fn title_name(&self) -> String {
        match &self.path {
            Some(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned()),
            None => "New Buffer".to_string(),
        }
    }
    /// True once the editor wants to exit.
    pub fn should_quit(&self) -> bool {
        self.quit
    }
    /// Text area size `(columns, rows)`.
    pub fn viewport(&self) -> (usize, usize) {
        self.viewport
    }
    /// Line ending used when saving.
    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    // ---- mutators ----

    /// Sets the text area size `(columns, rows)` and rescrolls.
    pub fn set_viewport(&mut self, cols: usize, rows: usize) {
        self.viewport = (cols.max(1), rows.max(1));
        self.scroll_to_cursor();
    }

    /// Sets the status message.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
    }

    /// Adjusts the scroll offsets so the cursor is visible.
    pub fn scroll_to_cursor(&mut self) {
        let (cols, rows) = self.viewport;
        let (l, c) = self.buffer.cursor();
        if l < self.top_line {
            self.top_line = l;
        } else if l >= self.top_line + rows {
            self.top_line = l + 1 - rows;
        }
        if c < self.left_col {
            self.left_col = c;
        } else if c >= self.left_col + cols {
            self.left_col = c + 1 - cols;
        }
    }

    fn record(&mut self, e: Option<Edit>) {
        if e.is_some() {
            self.history.record_opt(e);
            self.modified = true;
        }
    }

    /// Applies one action. Any action clears the previous status message.
    pub fn handle(&mut self, action: Action) {
        if let Action::Resize(w, h) = action {
            self.set_viewport(w as usize, (h as usize).saturating_sub(CHROME_ROWS));
            return;
        }
        self.status = None;
        if self.prompt.is_some() {
            self.handle_prompt(action);
        } else {
            self.handle_edit(action);
        }
        self.scroll_to_cursor();
    }

    fn handle_edit(&mut self, action: Action) {
        let rows = self.viewport.1;
        match action {
            Action::Insert(c) => {
                let e = self.buffer.insert_char(c);
                self.record(e);
            }
            Action::Tab => {
                let e = self.buffer.insert_char('\t');
                self.record(e);
            }
            Action::Newline => {
                let e = self.buffer.newline();
                self.record(e);
            }
            Action::Backspace => {
                let e = self.buffer.backspace();
                self.record(e);
            }
            Action::Delete => {
                let e = self.buffer.delete();
                self.record(e);
            }
            Action::Cut => {
                let e = self.buffer.cut();
                self.record(e);
            }
            Action::Paste => {
                let e = self.buffer.paste();
                self.record(e);
            }
            Action::Left => self.buffer.move_left(),
            Action::Right => self.buffer.move_right(),
            Action::Up => self.buffer.move_up(),
            Action::Down => self.buffer.move_down(),
            Action::Home => self.buffer.home(),
            Action::End => self.buffer.end(),
            Action::PageUp => self.buffer.page_up(rows),
            Action::PageDown => self.buffer.page_down(rows),
            Action::Undo => {
                if self.history.undo(&mut self.buffer) {
                    self.modified = true;
                } else {
                    self.set_status("Nothing to undo");
                }
            }
            Action::Redo => {
                if self.history.redo(&mut self.buffer) {
                    self.modified = true;
                } else {
                    self.set_status("Nothing to redo");
                }
            }
            Action::WriteOut => self.open_write_out(false),
            Action::Exit => {
                if self.modified {
                    self.prompt = Some(Prompt {
                        kind: PromptKind::SaveModified,
                        text: String::new(),
                        quit_after: true,
                    });
                } else {
                    self.quit = true;
                }
            }
            Action::Cancel | Action::Resize(..) => {}
        }
    }

    fn open_write_out(&mut self, quit_after: bool) {
        let text = self
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.prompt = Some(Prompt {
            kind: PromptKind::WriteOut,
            text,
            quit_after,
        });
    }

    fn handle_prompt(&mut self, action: Action) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        if action == Action::Cancel {
            self.prompt = None;
            self.set_status("Cancelled");
            return;
        }
        match prompt.kind {
            PromptKind::SaveModified => match action {
                Action::Insert('y' | 'Y') => self.open_write_out(true),
                Action::Insert('n' | 'N') => {
                    self.prompt = None;
                    self.quit = true;
                }
                _ => {}
            },
            PromptKind::WriteOut => match action {
                Action::Insert(c) => prompt.text.push(c),
                Action::Tab => prompt.text.push('\t'),
                Action::Backspace => {
                    prompt.text.pop();
                }
                Action::Newline => {
                    let name = prompt.text.trim().to_string();
                    let quit_after = prompt.quit_after;
                    self.prompt = None;
                    if name.is_empty() {
                        self.set_status("Cancelled");
                    } else {
                        self.write_to(PathBuf::from(name), quit_after);
                    }
                }
                _ => {}
            },
        }
    }

    fn write_to(&mut self, path: PathBuf, quit_after: bool) {
        let n = self.buffer.line_count();
        match fileio::save(&path, self.buffer.lines(), self.line_ending, self.final_newline) {
            Ok(()) => {
                self.path = Some(path);
                self.modified = false;
                self.set_status(format!("Wrote {n} lines"));
                if quit_after {
                    self.quit = true;
                }
            }
            Err(e) => {
                self.set_status(format!("Error writing {}: {e}", path.display()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nanorust-ed-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn type_str(ed: &mut Editor, s: &str) {
        for c in s.chars() {
            ed.handle(Action::Insert(c));
        }
    }

    #[test]
    fn edit_save_exit() {
        let d = tmpdir("save");
        let p = d.join("a.txt");
        let mut ed = Editor::open(&p, None).unwrap();
        assert_eq!(ed.status(), Some("New File"));
        type_str(&mut ed, "hi");
        ed.handle(Action::Newline);
        type_str(&mut ed, "yo");
        assert!(ed.modified());
        ed.handle(Action::WriteOut);
        assert_eq!(ed.prompt_kind(), Some(PromptKind::WriteOut));
        assert_eq!(ed.prompt_text(), Some(p.to_str().unwrap()));
        ed.handle(Action::Newline);
        assert_eq!(ed.status(), Some("Wrote 2 lines"));
        assert!(!ed.modified());
        assert_eq!(fs::read_to_string(&p).unwrap(), "hi\nyo\n");
        ed.handle(Action::Exit);
        assert!(ed.should_quit());
        assert_eq!(ed.title_name(), "a.txt");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn exit_unsaved_yes_saves_then_quits() {
        let d = tmpdir("yes");
        let p = d.join("b.txt");
        let mut ed = Editor::open(&p, None).unwrap();
        type_str(&mut ed, "x");
        ed.handle(Action::Exit);
        assert_eq!(ed.prompt_kind(), Some(PromptKind::SaveModified));
        assert!(!ed.should_quit());
        ed.handle(Action::Insert('y'));
        assert_eq!(ed.prompt_kind(), Some(PromptKind::WriteOut));
        ed.handle(Action::Newline);
        assert!(ed.should_quit());
        assert_eq!(fs::read_to_string(&p).unwrap(), "x\n");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn exit_unsaved_no_and_cancel() {
        let mut ed = Editor::new();
        type_str(&mut ed, "x");
        ed.handle(Action::Exit);
        ed.handle(Action::Cancel);
        assert_eq!(ed.prompt_kind(), None);
        assert!(!ed.should_quit());
        assert!(ed.modified());
        ed.handle(Action::Exit);
        ed.handle(Action::Insert('n'));
        assert!(ed.should_quit());
    }

    #[test]
    fn cancel_filename_prompt() {
        let mut ed = Editor::new();
        type_str(&mut ed, "x");
        ed.handle(Action::Exit);
        ed.handle(Action::Insert('Y'));
        type_str(&mut ed, "abc");
        ed.handle(Action::Backspace);
        assert_eq!(ed.prompt_text(), Some("ab"));
        ed.handle(Action::Cancel);
        assert_eq!(ed.prompt_kind(), None);
        assert!(!ed.should_quit());
        assert!(ed.modified());
    }

    #[test]
    fn save_error_message() {
        let d = tmpdir("err");
        let p = d.join("missing-dir").join("c.txt");
        let mut ed = Editor::new();
        type_str(&mut ed, "x");
        ed.handle(Action::WriteOut);
        type_str(&mut ed, p.to_str().unwrap());
        ed.handle(Action::Newline);
        assert!(ed.status().unwrap().starts_with("Error writing"));
        assert!(ed.modified());
        assert!(!ed.should_quit());
        assert!(ed.path().is_none());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn undo_redo_and_modified() {
        let mut ed = Editor::new();
        assert!(!ed.modified());
        type_str(&mut ed, "ab");
        assert!(ed.modified());
        ed.handle(Action::Undo);
        assert_eq!(ed.buffer().to_text(), "a");
        ed.handle(Action::Undo);
        assert_eq!(ed.buffer().to_text(), "");
        ed.handle(Action::Undo);
        assert_eq!(ed.status(), Some("Nothing to undo"));
        ed.handle(Action::Redo);
        assert_eq!(ed.buffer().to_text(), "a");
        ed.handle(Action::Newline);
        ed.handle(Action::Redo);
        assert_eq!(ed.status(), Some("Nothing to redo"));
    }

    #[test]
    fn cut_paste_and_movement_not_modifying() {
        let d = tmpdir("cut");
        let p = d.join("d.txt");
        fs::write(&p, "one\ntwo\n").unwrap();
        let mut ed = Editor::open(&p, Some(2)).unwrap();
        assert_eq!(ed.status(), Some("Read 2 lines"));
        assert_eq!(ed.buffer().cursor().0, 1);
        ed.handle(Action::Up);
        ed.handle(Action::End);
        ed.handle(Action::Home);
        assert!(!ed.modified());
        ed.handle(Action::Cut);
        assert!(ed.modified());
        assert_eq!(ed.buffer().to_text(), "two");
        ed.handle(Action::Paste);
        assert_eq!(ed.buffer().to_text(), "one\ntwo");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn scrolling_follows_cursor() {
        let mut ed = Editor::new();
        ed.handle(Action::Resize(40, 14));
        assert_eq!(ed.viewport(), (40, 10));
        for _ in 0..30 {
            ed.handle(Action::Newline);
        }
        assert_eq!(ed.top_line, 21);
        ed.handle(Action::PageUp);
        assert!(ed.top_line <= ed.buffer().cursor().0);
        for _ in 0..50 {
            ed.handle(Action::Insert('x'));
        }
        assert!(ed.left_col > 0);
    }
}
