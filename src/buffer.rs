//! Text buffer: lines, cursor and editing operations.
//!
//! The buffer is a `Vec<String>` of lines (never empty: an empty buffer has one
//! empty line). Positions are `(line, column)` pairs, both zero-based, where the
//! column counts Unicode scalar values (`char`s), not bytes, so every operation is
//! UTF-8 safe.
//!
//! Every mutating operation returns an [`Edit`] (or `None` when nothing changed).
//! An `Edit` can be re-applied with [`Buffer::apply`] and reversed with
//! [`Edit::inverse`], which is all an undo layer needs.

/// A zero-based `(line, char column)` position.
pub type Pos = (usize, usize);

/// A primitive text change. Text may contain `'\n'`, which spans lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// `text` was inserted at `pos`. Applying leaves the cursor after the text.
    Insert { pos: Pos, text: String },
    /// `text` was deleted starting at `pos`. Applying leaves the cursor at `pos`.
    Delete { pos: Pos, text: String },
}

impl Edit {
    /// The edit that undoes this one.
    pub fn inverse(&self) -> Edit {
        match self {
            Edit::Insert { pos, text } => Edit::Delete {
                pos: *pos,
                text: text.clone(),
            },
            Edit::Delete { pos, text } => Edit::Insert {
                pos: *pos,
                text: text.clone(),
            },
        }
    }
}

/// Position reached by walking over `text` starting at `pos`.
fn advance(pos: Pos, text: &str) -> Pos {
    let mut line = pos.0;
    let mut col = pos.1;
    for c in text.chars() {
        if c == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Byte offset of char column `col` in `s` (clamped to the end).
fn byte_idx(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map_or(s.len(), |(i, _)| i)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Line buffer with cursor, selection mark and cut buffer.
#[derive(Debug, Clone)]
pub struct Buffer {
    lines: Vec<String>,
    cursor: Pos,
    mark: Option<Pos>,
    /// Remembered column for consecutive vertical moves.
    goal_col: Option<usize>,
    clipboard: String,
    /// Whether the clipboard holds whole lines (pasted above the cursor line).
    clipboard_lines: bool,
    /// True if the previous operation was a line cut (so the next one appends).
    cut_chain: bool,
}

impl Default for Buffer {
    fn default() -> Self {
        Self::new()
    }
}

impl Buffer {
    /// An empty buffer: one empty line, cursor at the origin.
    pub fn new() -> Self {
        Buffer {
            lines: vec![String::new()],
            cursor: (0, 0),
            mark: None,
            goal_col: None,
            clipboard: String::new(),
            clipboard_lines: false,
            cut_chain: false,
        }
    }

    /// Builds a buffer from text, split on `'\n'` (`"a\n"` gives `["a", ""]`).
    /// A `'\r'` before a newline is kept as part of the line.
    pub fn from_text(text: &str) -> Self {
        let mut b = Self::new();
        b.lines = text.split('\n').map(str::to_string).collect();
        b
    }

    /// The whole text joined with `'\n'`.
    pub fn to_text(&self) -> String {
        self.lines.join("\n")
    }

    /// All lines.
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Number of lines (at least 1).
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Line `i`, if it exists.
    pub fn line(&self, i: usize) -> Option<&str> {
        self.lines.get(i).map(String::as_str)
    }

    /// Cursor position `(line, char column)`.
    pub fn cursor(&self) -> Pos {
        self.cursor
    }

    /// Moves the cursor, clamping to the text.
    pub fn set_cursor(&mut self, pos: Pos) {
        self.reset();
        self.cursor = self.clamp(pos);
    }

    /// The cut buffer contents.
    pub fn clipboard(&self) -> &str {
        &self.clipboard
    }

    fn reset(&mut self) {
        self.goal_col = None;
        self.cut_chain = false;
    }

    fn line_len(&self, l: usize) -> usize {
        self.lines.get(l).map_or(0, |s| char_len(s))
    }

    fn clamp(&self, pos: Pos) -> Pos {
        let l = pos.0.min(self.lines.len() - 1);
        (l, pos.1.min(self.line_len(l)))
    }

    // ---- primitives -------------------------------------------------------

    fn insert_text(&mut self, pos: Pos, text: &str) -> Pos {
        let (l, c) = self.clamp(pos);
        let b = byte_idx(&self.lines[l], c);
        let tail = self.lines[l].split_off(b);
        let mut parts = text.split('\n');
        if let Some(first) = parts.next() {
            self.lines[l].push_str(first);
        }
        let rest: Vec<&str> = parts.collect();
        let mut end = (l, c + char_len(text.split('\n').next().unwrap_or("")));
        for (i, p) in rest.iter().enumerate() {
            self.lines.insert(l + 1 + i, (*p).to_string());
            end = (l + 1 + i, char_len(p));
        }
        self.lines[end.0].push_str(&tail);
        end
    }

    fn delete_range(&mut self, a: Pos, b: Pos) {
        let a = self.clamp(a);
        let b = self.clamp(b);
        if b <= a {
            return;
        }
        let bi = byte_idx(&self.lines[b.0], b.1);
        let tail = self.lines[b.0].split_off(bi);
        let ai = byte_idx(&self.lines[a.0], a.1);
        self.lines[a.0].truncate(ai);
        self.lines[a.0].push_str(&tail);
        self.lines.drain(a.0 + 1..=b.0);
    }

    fn text_between(&self, a: Pos, b: Pos) -> String {
        let a = self.clamp(a);
        let b = self.clamp(b);
        if b <= a {
            return String::new();
        }
        let slice = |l: usize, from: usize, to: usize| -> String {
            self.lines[l].chars().skip(from).take(to - from).collect()
        };
        if a.0 == b.0 {
            return slice(a.0, a.1, b.1);
        }
        let mut out = slice(a.0, a.1, self.line_len(a.0));
        for l in a.0 + 1..b.0 {
            out.push('\n');
            out.push_str(&self.lines[l]);
        }
        out.push('\n');
        out.push_str(&slice(b.0, 0, b.1));
        out
    }

    /// Applies an edit (for redo, or for undo with `edit.inverse()`), moving the
    /// cursor as documented on [`Edit`]. Clears the selection mark.
    pub fn apply(&mut self, edit: &Edit) {
        self.reset();
        self.mark = None;
        match edit {
            Edit::Insert { pos, text } => {
                self.cursor = self.insert_text(*pos, text);
            }
            Edit::Delete { pos, text } => {
                let pos = self.clamp(*pos);
                self.delete_range(pos, advance(pos, text));
                self.cursor = pos;
            }
        }
    }

    fn do_edit(&mut self, e: Edit) -> Option<Edit> {
        self.apply(&e);
        Some(e)
    }

    fn delete_edit(&self, a: Pos, b: Pos) -> Option<Edit> {
        let text = self.text_between(a, b);
        if text.is_empty() {
            None
        } else {
            Some(Edit::Delete {
                pos: self.clamp(a),
                text,
            })
        }
    }

    // ---- editing ----------------------------------------------------------

    /// Inserts a string at the cursor (`'\n'` splits lines; `"\r\n"` is
    /// normalized to `'\n'`). Returns `None` for an empty string.
    pub fn insert_str(&mut self, s: &str) -> Option<Edit> {
        if s.is_empty() {
            return None;
        }
        let text = s.replace("\r\n", "\n");
        self.do_edit(Edit::Insert {
            pos: self.cursor,
            text,
        })
    }

    /// Inserts one char at the cursor.
    pub fn insert_char(&mut self, c: char) -> Option<Edit> {
        let mut buf = [0u8; 4];
        self.insert_str(c.encode_utf8(&mut buf))
    }

    /// Splits the line at the cursor.
    pub fn newline(&mut self) -> Option<Edit> {
        self.insert_str("\n")
    }

    /// Deletes the char before the cursor (joining lines at column 0).
    pub fn backspace(&mut self) -> Option<Edit> {
        let (l, c) = self.cursor;
        let from = if c > 0 {
            (l, c - 1)
        } else if l > 0 {
            (l - 1, self.line_len(l - 1))
        } else {
            return None;
        };
        let e = self.delete_edit(from, self.cursor)?;
        self.do_edit(e)
    }

    /// Deletes the char under the cursor (joining lines at end of line).
    pub fn delete(&mut self) -> Option<Edit> {
        let (l, c) = self.cursor;
        let to = if c < self.line_len(l) {
            (l, c + 1)
        } else if l + 1 < self.lines.len() {
            (l + 1, 0)
        } else {
            return None;
        };
        let e = self.delete_edit(self.cursor, to)?;
        self.do_edit(e)
    }

    // ---- movement ---------------------------------------------------------

    /// Left one char, wrapping to the end of the previous line.
    pub fn move_left(&mut self) {
        self.reset();
        let (l, c) = self.cursor;
        if c > 0 {
            self.cursor = (l, c - 1);
        } else if l > 0 {
            self.cursor = (l - 1, self.line_len(l - 1));
        }
    }

    /// Right one char, wrapping to the start of the next line.
    pub fn move_right(&mut self) {
        self.reset();
        let (l, c) = self.cursor;
        if c < self.line_len(l) {
            self.cursor = (l, c + 1);
        } else if l + 1 < self.lines.len() {
            self.cursor = (l + 1, 0);
        }
    }

    fn move_vertical(&mut self, target: usize) {
        self.cut_chain = false;
        let goal = *self.goal_col.get_or_insert(self.cursor.1);
        let l = target.min(self.lines.len() - 1);
        self.cursor = (l, goal.min(self.line_len(l)));
    }

    /// Up one line, keeping the desired column (clamped to the line length).
    pub fn move_up(&mut self) {
        self.move_vertical(self.cursor.0.saturating_sub(1));
    }

    /// Down one line, keeping the desired column (clamped to the line length).
    pub fn move_down(&mut self) {
        self.move_vertical(self.cursor.0 + 1);
    }

    /// Up by `height` lines (at least 1).
    pub fn page_up(&mut self, height: usize) {
        self.move_vertical(self.cursor.0.saturating_sub(height.max(1)));
    }

    /// Down by `height` lines (at least 1).
    pub fn page_down(&mut self, height: usize) {
        self.move_vertical(self.cursor.0.saturating_add(height.max(1)));
    }

    /// Start of the next word (wrapping to the next line at end of line).
    pub fn word_right(&mut self) {
        self.reset();
        let (l, c) = self.cursor;
        let chars: Vec<char> = self.lines[l].chars().collect();
        if c >= chars.len() {
            if l + 1 < self.lines.len() {
                self.cursor = (l + 1, 0);
            }
            return;
        }
        let mut i = c;
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        while i < chars.len() && !is_word(chars[i]) {
            i += 1;
        }
        self.cursor = (l, i);
    }

    /// Start of the previous word (wrapping to the previous line at column 0).
    pub fn word_left(&mut self) {
        self.reset();
        let (l, c) = self.cursor;
        if c == 0 {
            if l > 0 {
                self.cursor = (l - 1, self.line_len(l - 1));
            }
            return;
        }
        let chars: Vec<char> = self.lines[l].chars().collect();
        let mut i = c.min(chars.len());
        while i > 0 && !is_word(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(chars[i - 1]) {
            i -= 1;
        }
        self.cursor = (l, i);
    }

    /// Start of the current line.
    pub fn home(&mut self) {
        self.reset();
        self.cursor.1 = 0;
    }

    /// End of the current line.
    pub fn end(&mut self) {
        self.reset();
        self.cursor.1 = self.line_len(self.cursor.0);
    }

    /// Start of the file.
    pub fn file_start(&mut self) {
        self.reset();
        self.cursor = (0, 0);
    }

    /// End of the file.
    pub fn file_end(&mut self) {
        self.reset();
        let l = self.lines.len() - 1;
        self.cursor = (l, self.line_len(l));
    }

    /// Goes to a one-based line and optional one-based column, clamping both
    /// (0 counts as 1).
    pub fn goto(&mut self, line: usize, col: Option<usize>) {
        self.reset();
        let l = line.saturating_sub(1).min(self.lines.len() - 1);
        let c = col.map_or(0, |c| c.saturating_sub(1));
        self.cursor = (l, c.min(self.line_len(l)));
    }

    // ---- selection --------------------------------------------------------

    /// Sets the mark at the cursor.
    pub fn set_mark(&mut self) {
        self.mark = Some(self.cursor);
    }

    /// Clears the mark.
    pub fn clear_mark(&mut self) {
        self.mark = None;
    }

    /// The mark, if set.
    pub fn mark(&self) -> Option<Pos> {
        self.mark
    }

    /// Ordered `(start, end)` of the selection; `None` if no mark or it is empty.
    pub fn selection_range(&self) -> Option<(Pos, Pos)> {
        let m = self.clamp(self.mark?);
        if m == self.cursor {
            return None;
        }
        Some(if m < self.cursor {
            (m, self.cursor)
        } else {
            (self.cursor, m)
        })
    }

    /// The selected text, if any.
    pub fn selected_text(&self) -> Option<String> {
        let (a, b) = self.selection_range()?;
        Some(self.text_between(a, b))
    }

    // ---- cut / copy / paste ----------------------------------------------

    /// Copies the selection to the cut buffer and clears the mark.
    /// Returns whether anything was copied.
    pub fn copy(&mut self) -> bool {
        self.reset();
        let text = self.selected_text();
        self.mark = None;
        match text {
            Some(t) => {
                self.clipboard = t;
                self.clipboard_lines = false;
                true
            }
            None => false,
        }
    }

    /// Cuts the selection if there is one, otherwise the current line.
    pub fn cut(&mut self) -> Option<Edit> {
        if self.selection_range().is_some() {
            self.cut_selection()
        } else {
            self.mark = None;
            self.cut_line()
        }
    }

    /// Cuts the selection into the cut buffer.
    pub fn cut_selection(&mut self) -> Option<Edit> {
        let (a, b) = self.selection_range()?;
        let e = self.delete_edit(a, b)?;
        if let Edit::Delete { text, .. } = &e {
            self.clipboard = text.clone();
            self.clipboard_lines = false;
        }
        self.do_edit(e)
    }

    /// Cuts the current line. Consecutive line cuts accumulate in the cut
    /// buffer, as in nano. The only remaining line is emptied, not removed.
    pub fn cut_line(&mut self) -> Option<Edit> {
        let chained = self.cut_chain;
        let l = self.cursor.0;
        let text = format!("{}\n", self.lines[l]);
        let edit = if l + 1 < self.lines.len() {
            Some(Edit::Delete {
                pos: (l, 0),
                text: text.clone(),
            })
        } else if l > 0 {
            self.delete_edit((l - 1, self.line_len(l - 1)), (l, self.line_len(l)))
        } else {
            self.delete_edit((l, 0), (l, self.line_len(l)))
        };
        let edit = edit?;
        if chained && self.clipboard_lines {
            self.clipboard.push_str(&text);
        } else {
            self.clipboard = text;
            self.clipboard_lines = true;
        }
        let e = self.do_edit(edit);
        self.cut_chain = true;
        e
    }

    /// Cuts from the cursor to the end of the line (the newline if already there).
    pub fn cut_to_eol(&mut self) -> Option<Edit> {
        let (l, c) = self.cursor;
        let to = if c < self.line_len(l) {
            (l, self.line_len(l))
        } else if l + 1 < self.lines.len() {
            (l + 1, 0)
        } else {
            return None;
        };
        let e = self.delete_edit(self.cursor, to)?;
        if let Edit::Delete { text, .. } = &e {
            self.clipboard = text.clone();
            self.clipboard_lines = false;
        }
        self.do_edit(e)
    }

    /// Pastes the cut buffer: whole lines go above the cursor line, other text
    /// at the cursor. Returns `None` if the cut buffer is empty.
    pub fn paste(&mut self) -> Option<Edit> {
        if self.clipboard.is_empty() {
            return None;
        }
        let pos = if self.clipboard_lines {
            (self.cursor.0, 0)
        } else {
            self.cursor
        };
        let text = self.clipboard.clone();
        self.do_edit(Edit::Insert { pos, text })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(s: &str) -> Buffer {
        Buffer::from_text(s)
    }

    #[test]
    fn empty_buffer() {
        let mut b = Buffer::new();
        assert_eq!(b.lines(), [""]);
        assert!(b.backspace().is_none());
        assert!(b.delete().is_none());
        assert!(b.cut().is_none());
        assert!(b.paste().is_none());
        b.move_left();
        b.move_up();
        b.word_left();
        b.word_right();
        b.page_down(10);
        assert_eq!(b.cursor(), (0, 0));
    }

    #[test]
    fn insert_and_newline() {
        let mut b = Buffer::new();
        b.insert_char('a');
        b.insert_char('é');
        b.newline();
        b.insert_str("x\ny");
        assert_eq!(b.to_text(), "aé\nx\ny");
        assert_eq!(b.cursor(), (2, 1));
        b.set_cursor((0, 1));
        b.newline();
        assert_eq!(b.to_text(), "a\né\nx\ny");
        assert_eq!(b.cursor(), (1, 0));
    }

    #[test]
    fn multibyte_editing() {
        let mut b = buf("é世🎉x");
        b.move_right();
        b.move_right();
        assert_eq!(b.cursor(), (0, 2));
        b.insert_char('🎉');
        assert_eq!(b.to_text(), "é世🎉🎉x");
        b.backspace();
        b.backspace();
        assert_eq!(b.to_text(), "é🎉x");
        b.set_cursor((0, 0));
        b.delete();
        assert_eq!(b.to_text(), "🎉x");
        b.end();
        assert_eq!(b.cursor(), (0, 2));
    }

    #[test]
    fn backspace_delete_join() {
        let mut b = buf("ab\ncd");
        b.set_cursor((1, 0));
        let e = b.backspace().unwrap();
        assert_eq!(b.to_text(), "abcd");
        assert_eq!(b.cursor(), (0, 2));
        b.apply(&e.inverse());
        assert_eq!(b.to_text(), "ab\ncd");
        b.set_cursor((0, 2));
        b.delete();
        assert_eq!(b.to_text(), "abcd");
        b.file_end();
        assert!(b.delete().is_none());
    }

    #[test]
    fn movement_wrap_and_clamp() {
        let mut b = buf("abcdef\nx\nabcdef");
        b.set_cursor((0, 5));
        b.move_down();
        assert_eq!(b.cursor(), (1, 1));
        b.move_down();
        assert_eq!(b.cursor(), (2, 5)); // goal column restored
        b.move_up();
        b.move_up();
        b.move_up();
        assert_eq!(b.cursor(), (0, 5));
        b.set_cursor((0, 6));
        b.move_right();
        assert_eq!(b.cursor(), (1, 0));
        b.move_left();
        assert_eq!(b.cursor(), (0, 6));
        b.set_cursor((9, 9));
        assert_eq!(b.cursor(), (2, 6));
    }

    #[test]
    fn words() {
        let mut b = buf("foo  bar_1, é世\nnext");
        b.word_right();
        assert_eq!(b.cursor(), (0, 5));
        b.word_right();
        assert_eq!(b.cursor(), (0, 12));
        b.word_right();
        assert_eq!(b.cursor(), (0, 14));
        b.word_right();
        assert_eq!(b.cursor(), (1, 0));
        b.word_left();
        assert_eq!(b.cursor(), (0, 14));
        b.word_left();
        assert_eq!(b.cursor(), (0, 12));
        b.word_left();
        assert_eq!(b.cursor(), (0, 5));
        b.word_left();
        assert_eq!(b.cursor(), (0, 0));
    }

    #[test]
    fn home_end_pages_goto() {
        let text: Vec<String> = (0..20).map(|i| format!("l{i}")).collect();
        let mut b = buf(&text.join("\n"));
        b.page_down(8);
        assert_eq!(b.cursor().0, 8);
        b.page_down(100);
        assert_eq!(b.cursor().0, 19);
        b.page_up(5);
        assert_eq!(b.cursor().0, 14);
        b.page_up(100);
        assert_eq!(b.cursor().0, 0);
        b.end();
        assert_eq!(b.cursor(), (0, 2));
        b.home();
        assert_eq!(b.cursor(), (0, 0));
        b.file_end();
        assert_eq!(b.cursor(), (19, 3));
        b.file_start();
        assert_eq!(b.cursor(), (0, 0));
        b.goto(5, Some(2));
        assert_eq!(b.cursor(), (4, 1));
        b.goto(0, Some(99));
        assert_eq!(b.cursor(), (0, 2));
        b.goto(99, None);
        assert_eq!(b.cursor(), (19, 0));
    }

    #[test]
    fn selection_and_copy() {
        let mut b = buf("héllo\n世界 x");
        b.set_cursor((0, 1));
        b.set_mark();
        assert_eq!(b.selected_text(), None);
        b.set_cursor((1, 2));
        assert_eq!(b.selection_range(), Some(((0, 1), (1, 2))));
        assert_eq!(b.selected_text().unwrap(), "éllo\n世界");
        assert!(b.copy());
        assert!(b.mark().is_none());
        assert_eq!(b.clipboard(), "éllo\n世界");
        b.file_end();
        b.paste();
        assert_eq!(b.to_text(), "héllo\n世界 xéllo\n世界");
    }

    #[test]
    fn cut_selection_backwards_and_undo() {
        let mut b = buf("abc🎉def");
        b.set_cursor((0, 5));
        b.set_mark();
        b.set_cursor((0, 2));
        // set_cursor keeps the mark
        let e = b.cut().unwrap();
        assert_eq!(b.to_text(), "abef");
        assert_eq!(b.clipboard(), "c🎉d");
        b.apply(&e.inverse());
        assert_eq!(b.to_text(), "abc🎉def");
        b.apply(&e);
        assert_eq!(b.to_text(), "abef");
    }

    #[test]
    fn cut_lines_accumulate() {
        let mut b = buf("one\ntwo\nthree\nfour");
        b.cut_line();
        b.cut_line();
        assert_eq!(b.clipboard(), "one\ntwo\n");
        assert_eq!(b.to_text(), "three\nfour");
        b.move_down();
        b.cut_line();
        assert_eq!(b.clipboard(), "four\n"); // chain broken by movement
        assert_eq!(b.to_text(), "three");
        b.file_start();
        b.paste();
        assert_eq!(b.to_text(), "four\nthree");
    }

    #[test]
    fn cut_last_line_and_empty() {
        let mut b = buf("only");
        let e = b.cut_line().unwrap();
        assert_eq!(b.lines(), [""]);
        assert!(b.cut_line().is_none());
        b.apply(&e.inverse());
        assert_eq!(b.to_text(), "only");
    }

    #[test]
    fn cut_to_eol() {
        let mut b = buf("héllo wörld\nnext");
        b.set_cursor((0, 6));
        b.cut_to_eol();
        assert_eq!(b.to_text(), "héllo \nnext");
        assert_eq!(b.clipboard(), "wörld");
        b.cut_to_eol();
        assert_eq!(b.to_text(), "héllo next");
        assert_eq!(b.clipboard(), "\n");
        b.file_end();
        assert!(b.cut_to_eol().is_none());
        b.paste();
        assert_eq!(b.to_text(), "héllo next\n");
    }

    #[test]
    fn edits_round_trip() {
        let mut b = buf("a世b");
        let mut log = Vec::new();
        b.move_right();
        log.extend(b.insert_str("🎉\nz"));
        log.extend(b.backspace());
        log.extend(b.newline());
        log.extend(b.delete());
        let after = b.to_text();
        for e in log.iter().rev() {
            b.apply(&e.inverse());
        }
        assert_eq!(b.to_text(), "a世b");
        for e in &log {
            b.apply(e);
        }
        assert_eq!(b.to_text(), after);
    }

    #[test]
    fn crlf_paste_normalized() {
        let mut b = Buffer::new();
        b.insert_str("a\r\nb");
        assert_eq!(b.to_text(), "a\nb");
    }
}
