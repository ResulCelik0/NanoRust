//! Editor state: buffer, history, file info, prompts, status and scrolling.
//!
//! The UI feeds [`Action`]s to [`Editor::handle`] and renders from the read
//! accessors. Nothing here touches the terminal.

use std::io;
use std::path::{Path, PathBuf};

use crate::buffer::{Buffer, Edit, Pos};
use crate::fileio::{self, LineEnding};
use crate::keys::Action;
use crate::search::{self, Match, Options};
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
    /// "Search:" query input.
    Search,
    /// "Replace:" query input (step one of replace).
    Replace,
    /// "Replace with:" replacement input (step two of replace).
    ReplaceWith,
    /// Per-match "Replace this instance?" confirmation (Y/N/A/Esc).
    ReplaceConfirm,
    /// "Enter line number, column number:" input.
    Goto,
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
    /// Last search/replace query (reused when the prompt is confirmed empty).
    last_query: String,
    /// Replacement text of a replace in progress.
    replacement: String,
    /// Replacements made by the replace in progress.
    replaced: usize,
    case_sensitive: bool,
    backward: bool,
    current_match: Option<Match>,
    help: bool,
    /// First visible buffer line.
    pub top_line: usize,
    /// First visible character column.
    pub left_col: usize,
    /// Text area size `(columns, rows)`.
    viewport: (usize, usize),
    tab_size: usize,
    tabs_to_spaces: bool,
    line_numbers: bool,
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
            last_query: String::new(),
            replacement: String::new(),
            replaced: 0,
            case_sensitive: false,
            backward: false,
            current_match: None,
            help: false,
            top_line: 0,
            left_col: 0,
            viewport: (80, 24 - CHROME_ROWS),
            tab_size: 8,
            tabs_to_spaces: false,
            line_numbers: false,
        }
    }

    // ---- display options ----

    /// Applies command-line display options.
    pub fn configure(&mut self, tab_size: Option<usize>, tabs_to_spaces: bool, numbers: bool) {
        self.tab_size = tab_size.unwrap_or(8).max(1);
        self.tabs_to_spaces = tabs_to_spaces;
        self.line_numbers = numbers;
    }

    /// Tab stop width in columns.
    pub fn tab_size(&self) -> usize {
        self.tab_size
    }

    /// Whether Tab inserts spaces.
    pub fn tabs_to_spaces(&self) -> bool {
        self.tabs_to_spaces
    }

    /// Whether the line-number gutter is shown.
    pub fn line_numbers(&self) -> bool {
        self.line_numbers
    }

    /// Whether the full-screen help is showing (any key dismisses it).
    pub fn help_visible(&self) -> bool {
        self.help
    }

    /// Shows or hides the help screen.
    pub fn set_help_visible(&mut self, v: bool) {
        self.help = v;
    }

    /// Selection as ordered `(start, end)` char positions, if any.
    pub fn selection_range(&self) -> Option<(Pos, Pos)> {
        self.buffer.selection_range()
    }

    /// Current search match to highlight, as ordered `(start, end)` char positions.
    pub fn match_range(&self) -> Option<(Pos, Pos)> {
        self.current_match()
    }

    /// Sets (or clears) the highlighted search match.
    pub fn set_match_range(&mut self, r: Option<(Pos, Pos)>) {
        self.current_match = r.map(|(a, b)| Match {
            start: a,
            len: b.1.saturating_sub(a.1),
        });
    }

    #[cfg(test)]
    pub(crate) fn set_mark_for_test(&mut self) {
        self.buffer.set_mark();
    }

    fn insert_tab(&mut self) -> Option<Edit> {
        if !self.tabs_to_spaces {
            return self.buffer.insert_char('\t');
        }
        let (l, c) = self.buffer.cursor();
        let line = self.buffer.line(l).unwrap_or("");
        let mut col = 0;
        for ch in line.chars().take(c) {
            col += if ch == '\t' {
                self.tab_size - col % self.tab_size
            } else {
                unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1)
            };
        }
        let n = self.tab_size - col % self.tab_size;
        self.buffer.insert_str(&" ".repeat(n))
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
    pub fn prompt_label(&self) -> Option<String> {
        let kind = self.prompt_kind()?;
        let toggles = |base: &str| {
            let mut l = base.to_string();
            if self.case_sensitive {
                l.push_str(" [Case Sensitive]");
            }
            if self.backward {
                l.push_str(" [Backwards]");
            }
            l
        };
        Some(match kind {
            PromptKind::WriteOut => "File Name to Write".to_string(),
            PromptKind::SaveModified => "Save modified buffer? (Y/N/Ctrl+C)".to_string(),
            PromptKind::Search => toggles("Search"),
            PromptKind::Replace => toggles("Replace"),
            PromptKind::ReplaceWith => "Replace with".to_string(),
            PromptKind::ReplaceConfirm => "Replace this instance? (Y/N/A/Esc)".to_string(),
            PromptKind::Goto => "Enter line number, column number".to_string(),
        })
    }
    /// The current search match as an ordered `(start, end)` range, for highlighting.
    pub fn current_match(&self) -> Option<(Pos, Pos)> {
        self.current_match.map(|m| (m.start, m.end()))
    }
    /// Where the mark was set, if it is.
    pub fn mark_anchor(&self) -> Option<Pos> {
        self.buffer.mark()
    }
    /// Whether searches are case sensitive.
    pub fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }
    /// Whether searches go backwards.
    pub fn backward(&self) -> bool {
        self.backward
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
        if self.help {
            // Ctrl+G toggles; any other key also dismisses the help screen.
            self.help = false;
            return;
        }
        if self.prompt.is_some() {
            self.handle_prompt(action);
        } else {
            self.handle_edit(action);
        }
        self.scroll_to_cursor();
    }

    fn handle_edit(&mut self, action: Action) {
        let rows = self.viewport.1;
        if action != Action::RepeatSearch {
            self.current_match = None;
        }
        match action {
            Action::Insert(c) => {
                let e = self.buffer.insert_char(c);
                self.record(e);
            }
            Action::Tab => {
                let e = self.insert_tab();
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
            Action::Search => self.open_prompt(PromptKind::Search),
            Action::Replace => self.open_prompt(PromptKind::Replace),
            Action::RepeatSearch => self.repeat_search(),
            Action::Goto => self.open_prompt(PromptKind::Goto),
            Action::Cancel => self.show_position(),
            Action::Mark => {
                if self.buffer.mark().is_some() {
                    self.buffer.clear_mark();
                    self.set_status("Mark Unset");
                } else {
                    self.buffer.set_mark();
                    self.set_status("Mark Set");
                }
            }
            Action::Help => self.help = true,
            Action::Escape
            | Action::ToggleCase
            | Action::ToggleBackwards
            | Action::Resize(..) => {}
        }
    }

    fn open_prompt(&mut self, kind: PromptKind) {
        self.prompt = Some(Prompt {
            kind,
            text: String::new(),
            quit_after: false,
        });
    }

    fn search_opts(&self) -> Options {
        Options {
            case_sensitive: self.case_sensitive,
            backward: self.backward,
            wrap: true,
        }
    }

    fn not_found(&mut self) {
        self.current_match = None;
        let q = self.last_query.clone();
        self.set_status(format!("\"{q}\" not found"));
    }

    /// Searches from the cursor for the last query and moves to the match.
    fn run_search(&mut self) {
        if self.last_query.is_empty() {
            self.set_status("No current search pattern");
            return;
        }
        let from = self.buffer.cursor();
        match search::find(&self.buffer, from, &self.last_query, self.search_opts()) {
            Some(m) => {
                let wrapped = if self.backward {
                    m.start >= from
                } else {
                    m.start <= from
                };
                self.buffer.set_cursor(m.start);
                self.current_match = Some(m);
                if wrapped {
                    self.set_status("Search Wrapped");
                }
            }
            None => self.not_found(),
        }
    }

    fn repeat_search(&mut self) {
        self.run_search();
    }

    /// Shows "line L/N (P%), col C/M (P%), char X/Y (P%)" in the status bar.
    fn show_position(&mut self) {
        let (l, c) = self.buffer.cursor();
        let lines = self.buffer.line_count();
        let line_chars = |i: usize| self.buffer.line(i).map_or(0, |s| s.chars().count());
        let total: usize = (0..lines).map(line_chars).sum::<usize>() + lines - 1;
        let before: usize = (0..l).map(|i| line_chars(i) + 1).sum::<usize>() + c;
        let cols = line_chars(l) + 1;
        let pct = |a: usize, b: usize| a * 100 / b.max(1);
        self.set_status(format!(
            "line {}/{} ({}%), col {}/{} ({}%), char {}/{} ({}%)",
            l + 1,
            lines,
            pct(l + 1, lines),
            c + 1,
            cols,
            pct(c + 1, cols),
            before + 1,
            total + 1,
            pct(before + 1, total + 1),
        ));
    }

    fn goto_from_text(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            self.set_status("Cancelled");
            return;
        }
        let (line_s, col_s) = match text.split_once(',') {
            Some((a, b)) => (a.trim(), Some(b.trim())),
            None => (text, None),
        };
        let line = if line_s.is_empty() && col_s.is_some() {
            Some(self.buffer.cursor().0 + 1)
        } else {
            line_s.parse::<usize>().ok()
        };
        let col = match col_s {
            None | Some("") => Some(None),
            Some(c) => c.parse::<usize>().ok().map(Some),
        };
        match (line, col) {
            (Some(l), Some(c)) => {
                self.buffer.goto(l, c);
            }
            _ => self.set_status("Invalid line or column number"),
        }
    }

    fn start_replace(&mut self) {
        let from = self.buffer.cursor();
        self.replaced = 0;
        self.find_replace_match(from);
        if self.current_match.is_none() {
            self.prompt = None;
            self.not_found();
        } else {
            self.open_prompt(PromptKind::ReplaceConfirm);
        }
    }

    /// Looks for the next match at or after `from` (no wrap) for a replace.
    fn find_replace_match(&mut self, from: Pos) {
        let opts = Options {
            backward: false,
            wrap: false,
            ..self.search_opts()
        };
        self.current_match = search::find_from(&self.buffer, from, &self.last_query, opts);
        if let Some(m) = self.current_match {
            self.buffer.set_cursor(m.start);
        }
    }

    fn finish_replace(&mut self) {
        self.prompt = None;
        self.current_match = None;
        let n = self.replaced;
        let s = if n == 1 { "" } else { "s" };
        self.set_status(format!("Replaced {n} occurrence{s}"));
    }

    fn replace_confirm(&mut self, all: bool) {
        let mut group = Vec::new();
        while let Some(m) = self.current_match {
            group.extend(search::replace_match(&mut self.buffer, &m, &self.replacement));
            self.replaced += 1;
            let from = self.buffer.cursor();
            self.find_replace_match(from);
            if !all {
                break;
            }
        }
        if !group.is_empty() {
            self.history.record_group(group);
            self.modified = true;
        }
        if self.current_match.is_none() {
            self.finish_replace();
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
        if matches!(action, Action::Cancel | Action::Escape) {
            if prompt.kind == PromptKind::ReplaceConfirm {
                self.finish_replace();
            } else {
                self.prompt = None;
                self.current_match = None;
                self.set_status("Cancelled");
            }
            return;
        }
        match prompt.kind {
            PromptKind::ReplaceConfirm => match action {
                Action::Insert('y' | 'Y') => self.replace_confirm(false),
                Action::Insert('a' | 'A') => self.replace_confirm(true),
                Action::Insert('n' | 'N') => {
                    if let Some(m) = self.current_match {
                        self.find_replace_match((m.start.0, m.start.1 + 1));
                    }
                    if self.current_match.is_none() {
                        self.finish_replace();
                    }
                }
                _ => {}
            },
            PromptKind::Search
            | PromptKind::Replace
            | PromptKind::ReplaceWith
            | PromptKind::Goto => self.handle_text_prompt(action),
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

    fn handle_text_prompt(&mut self, action: Action) {
        let Some(prompt) = self.prompt.as_mut() else {
            return;
        };
        let kind = prompt.kind;
        match action {
            Action::Insert(c) => prompt.text.push(c),
            Action::Backspace => {
                prompt.text.pop();
            }
            Action::ToggleCase if matches!(kind, PromptKind::Search | PromptKind::Replace) => {
                self.case_sensitive = !self.case_sensitive;
            }
            Action::ToggleBackwards
                if matches!(kind, PromptKind::Search | PromptKind::Replace) =>
            {
                self.backward = !self.backward;
            }
            Action::Newline => {
                let text = std::mem::take(&mut prompt.text);
                self.prompt = None;
                match kind {
                    PromptKind::Search | PromptKind::Replace => {
                        if !text.is_empty() {
                            self.last_query = text;
                        }
                        if self.last_query.is_empty() {
                            self.set_status("Cancelled");
                        } else if kind == PromptKind::Search {
                            self.run_search();
                        } else {
                            self.open_prompt(PromptKind::ReplaceWith);
                        }
                    }
                    PromptKind::ReplaceWith => {
                        self.replacement = text;
                        self.start_replace();
                    }
                    _ => self.goto_from_text(&text),
                }
            }
            _ => {}
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

    fn ed_with(text: &str) -> Editor {
        let mut ed = Editor::new();
        ed.buffer = Buffer::from_text(text);
        ed
    }

    fn search_for(ed: &mut Editor, q: &str) {
        ed.handle(Action::Search);
        type_str(ed, q);
        ed.handle(Action::Newline);
    }

    #[test]
    fn search_finds_and_highlights() {
        let mut ed = ed_with("one two\nthree two");
        search_for(&mut ed, "two");
        assert_eq!(ed.prompt_kind(), None);
        assert_eq!(ed.buffer().cursor(), (0, 4));
        assert_eq!(ed.current_match(), Some(((0, 4), (0, 7))));
        ed.handle(Action::RepeatSearch);
        assert_eq!(ed.buffer().cursor(), (1, 6));
        ed.handle(Action::RepeatSearch);
        assert_eq!(ed.buffer().cursor(), (0, 4));
        assert_eq!(ed.status(), Some("Search Wrapped"));
        ed.handle(Action::Right);
        assert_eq!(ed.current_match(), None);
    }

    #[test]
    fn search_empty_input_reuses_last_query() {
        let mut ed = ed_with("ab ab ab");
        search_for(&mut ed, "ab");
        assert_eq!(ed.buffer().cursor(), (0, 3));
        ed.handle(Action::Search);
        ed.handle(Action::Newline);
        assert_eq!(ed.buffer().cursor(), (0, 6));
    }

    #[test]
    fn search_empty_without_history_cancels() {
        let mut ed = ed_with("ab");
        ed.handle(Action::Search);
        ed.handle(Action::Newline);
        assert_eq!(ed.prompt_kind(), None);
        assert_eq!(ed.status(), Some("Cancelled"));
        ed.handle(Action::RepeatSearch);
        assert_eq!(ed.status(), Some("No current search pattern"));
    }

    #[test]
    fn search_not_found_and_cancel() {
        let mut ed = ed_with("abc");
        search_for(&mut ed, "zzz");
        assert_eq!(ed.status(), Some("\"zzz\" not found"));
        assert_eq!(ed.current_match(), None);
        ed.handle(Action::Search);
        type_str(&mut ed, "a");
        assert_eq!(ed.prompt_text(), Some("a"));
        ed.handle(Action::Backspace);
        assert_eq!(ed.prompt_text(), Some(""));
        ed.handle(Action::Cancel);
        assert_eq!(ed.prompt_kind(), None);
        assert_eq!(ed.status(), Some("Cancelled"));
        ed.handle(Action::Search);
        ed.handle(Action::Escape);
        assert_eq!(ed.prompt_kind(), None);
    }

    #[test]
    fn search_case_toggle_and_label() {
        let mut ed = ed_with("Foo foo");
        ed.handle(Action::Search);
        assert_eq!(ed.prompt_label().as_deref(), Some("Search"));
        ed.handle(Action::ToggleCase);
        assert_eq!(ed.prompt_label().as_deref(), Some("Search [Case Sensitive]"));
        type_str(&mut ed, "foo");
        ed.handle(Action::Newline);
        assert_eq!(ed.buffer().cursor(), (0, 4));
        assert!(ed.case_sensitive());
        ed.handle(Action::Search);
        ed.handle(Action::ToggleCase);
        type_str(&mut ed, "FOO");
        ed.handle(Action::Newline);
        assert_eq!(ed.buffer().cursor(), (0, 0));
    }

    #[test]
    fn search_backwards() {
        let mut ed = ed_with("x a x a x");
        ed.handle(Action::Search);
        ed.handle(Action::ToggleBackwards);
        assert_eq!(ed.prompt_label().as_deref(), Some("Search [Backwards]"));
        type_str(&mut ed, "x");
        ed.handle(Action::Newline);
        // Wraps to the last match from the start.
        assert_eq!(ed.buffer().cursor(), (0, 8));
        assert_eq!(ed.status(), Some("Search Wrapped"));
        ed.handle(Action::RepeatSearch);
        assert_eq!(ed.buffer().cursor(), (0, 4));
        assert_eq!(ed.status(), None);
    }

    #[test]
    fn replace_confirm_yes_no_all() {
        let mut ed = ed_with("a a a a");
        ed.handle(Action::Replace);
        assert_eq!(ed.prompt_kind(), Some(PromptKind::Replace));
        type_str(&mut ed, "a");
        ed.handle(Action::Newline);
        assert_eq!(ed.prompt_kind(), Some(PromptKind::ReplaceWith));
        type_str(&mut ed, "bb");
        ed.handle(Action::Newline);
        assert_eq!(ed.prompt_kind(), Some(PromptKind::ReplaceConfirm));
        assert_eq!(ed.current_match(), Some(((0, 0), (0, 1))));
        ed.handle(Action::Insert('y'));
        assert_eq!(ed.buffer().to_text(), "bb a a a");
        assert_eq!(ed.current_match(), Some(((0, 3), (0, 4))));
        ed.handle(Action::Insert('n'));
        assert_eq!(ed.current_match(), Some(((0, 5), (0, 6))));
        ed.handle(Action::Insert('x'));
        assert_eq!(ed.prompt_kind(), Some(PromptKind::ReplaceConfirm));
        ed.handle(Action::Insert('A'));
        assert_eq!(ed.prompt_kind(), None);
        assert_eq!(ed.buffer().to_text(), "bb a bb bb");
        assert_eq!(ed.status(), Some("Replaced 3 occurrences"));
        assert!(ed.modified());
        ed.handle(Action::Undo);
        assert_eq!(ed.buffer().to_text(), "bb a a a");
        ed.handle(Action::Undo);
        assert_eq!(ed.buffer().to_text(), "a a a a");
    }

    #[test]
    fn replace_cancel_and_empty_replacement() {
        let mut ed = ed_with("xax");
        ed.handle(Action::Replace);
        type_str(&mut ed, "x");
        ed.handle(Action::Newline);
        ed.handle(Action::Newline); // empty replacement
        ed.handle(Action::Insert('y'));
        assert_eq!(ed.buffer().to_text(), "ax");
        ed.handle(Action::Escape);
        assert_eq!(ed.prompt_kind(), None);
        assert_eq!(ed.status(), Some("Replaced 1 occurrence"));
        assert_eq!(ed.buffer().to_text(), "ax");
        // Ctrl+C in the second step cancels without changes.
        ed.handle(Action::Replace);
        ed.handle(Action::Newline);
        assert_eq!(ed.prompt_kind(), Some(PromptKind::ReplaceWith));
        ed.handle(Action::Cancel);
        assert_eq!(ed.prompt_kind(), None);
        assert_eq!(ed.buffer().to_text(), "ax");
    }

    #[test]
    fn replace_not_found_and_all_declined() {
        let mut ed = ed_with("abc");
        ed.handle(Action::Replace);
        type_str(&mut ed, "q");
        ed.handle(Action::Newline);
        ed.handle(Action::Newline);
        assert_eq!(ed.prompt_kind(), None);
        assert_eq!(ed.status(), Some("\"q\" not found"));
        assert!(!ed.modified());
        ed.handle(Action::Replace);
        type_str(&mut ed, "b");
        ed.handle(Action::Newline);
        type_str(&mut ed, "z");
        ed.handle(Action::Newline);
        ed.handle(Action::Insert('n'));
        assert_eq!(ed.prompt_kind(), None);
        assert_eq!(ed.status(), Some("Replaced 0 occurrences"));
        assert!(!ed.modified());
    }

    #[test]
    fn goto_line_and_column() {
        let mut ed = ed_with("abc\ndef\nghi");
        ed.handle(Action::Goto);
        assert_eq!(ed.prompt_kind(), Some(PromptKind::Goto));
        type_str(&mut ed, "2,3");
        ed.handle(Action::Newline);
        assert_eq!(ed.buffer().cursor(), (1, 2));
        ed.handle(Action::Goto);
        type_str(&mut ed, "3");
        ed.handle(Action::Newline);
        assert_eq!(ed.buffer().cursor(), (2, 0));
        ed.handle(Action::Goto);
        type_str(&mut ed, ",2");
        ed.handle(Action::Newline);
        assert_eq!(ed.buffer().cursor(), (2, 1));
        for bad in ["x", "1,y", "-1", "1,2,3"] {
            ed.handle(Action::Goto);
            type_str(&mut ed, bad);
            ed.handle(Action::Newline);
            assert_eq!(ed.status(), Some("Invalid line or column number"), "{bad}");
            assert_eq!(ed.buffer().cursor(), (2, 1));
        }
        ed.handle(Action::Goto);
        ed.handle(Action::Cancel);
        assert_eq!(ed.prompt_kind(), None);
        ed.handle(Action::Goto);
        ed.handle(Action::Newline);
        assert_eq!(ed.status(), Some("Cancelled"));
    }

    #[test]
    fn cursor_position_status() {
        let mut ed = ed_with("abc\ndef");
        ed.handle(Action::Down);
        ed.handle(Action::Right);
        ed.handle(Action::Cancel);
        assert_eq!(
            ed.status(),
            Some("line 2/2 (100%), col 2/4 (50%), char 6/8 (75%)")
        );
        assert_eq!(ed.prompt_kind(), None);
    }

    #[test]
    fn mark_toggle_and_selection() {
        let mut ed = ed_with("hello");
        assert_eq!(ed.mark_anchor(), None);
        ed.handle(Action::Mark);
        assert_eq!(ed.mark_anchor(), Some((0, 0)));
        assert_eq!(ed.status(), Some("Mark Set"));
        ed.handle(Action::Right);
        ed.handle(Action::Right);
        assert_eq!(ed.selection_range(), Some(((0, 0), (0, 2))));
        ed.handle(Action::Mark);
        assert_eq!(ed.mark_anchor(), None);
        assert_eq!(ed.selection_range(), None);
        ed.handle(Action::Mark);
        ed.handle(Action::Left);
        ed.handle(Action::Cut);
        assert_eq!(ed.buffer().to_text(), "hllo");
        ed.handle(Action::Paste);
        assert_eq!(ed.buffer().to_text(), "hello");
    }

    #[test]
    fn help_toggle() {
        let mut ed = ed_with("a");
        assert!(!ed.help_visible());
        ed.handle(Action::Help);
        assert!(ed.help_visible());
        ed.handle(Action::Help);
        assert!(!ed.help_visible());
        ed.handle(Action::Help);
        ed.handle(Action::Insert('z'));
        assert!(!ed.help_visible());
        assert_eq!(ed.buffer().to_text(), "a");
    }
}
