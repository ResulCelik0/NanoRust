//! Rich-text side data: character styles aligned with the buffer, and an image table.
//!
//! [`Rich`] keeps one [`Style`] per buffer char (newlines have no style slot). It is
//! updated by mirroring each [`Edit`] with [`Rich::apply`]. Because a plain `Edit::Delete`
//! does not carry the styles of the removed text, [`Rich::apply`] returns a [`RichEdit`]:
//! a resolved record that also holds the styles. Undo/redo store the `RichEdit` next to
//! the `Edit` and replay it with [`Rich::apply_rich`] / [`RichEdit::inverse`]; there is
//! no separate undo stack.

use crate::buffer::{Edit, Pos};

/// Character style bit flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Style(pub u8);

impl Style {
    pub const NONE: Style = Style(0);
    pub const BOLD: Style = Style(1);
    pub const ITALIC: Style = Style(2);
    pub const UNDERLINE: Style = Style(4);

    pub fn contains(self, other: Style) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn with(self, other: Style) -> Style {
        Style(self.0 | other.0)
    }
    pub fn without(self, other: Style) -> Style {
        Style(self.0 & !other.0)
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// An edit resolved with style data, so it can be replayed and inverted exactly.
/// `styles` are flat: one entry per char of the affected text, in order, with
/// `'\n'` entries holding `Style::NONE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RichEdit {
    Insert { pos: Pos, text: String, styles: Vec<Style> },
    Delete { pos: Pos, text: String, styles: Vec<Style> },
    /// Styles over `[start, start + before.len())` changed from `before` to `after`.
    SetStyle { start: Pos, before: Vec<Style>, after: Vec<Style> },
}

impl RichEdit {
    pub fn inverse(&self) -> RichEdit {
        match self {
            RichEdit::Insert { pos, text, styles } => RichEdit::Delete {
                pos: *pos,
                text: text.clone(),
                styles: styles.clone(),
            },
            RichEdit::Delete { pos, text, styles } => RichEdit::Insert {
                pos: *pos,
                text: text.clone(),
                styles: styles.clone(),
            },
            RichEdit::SetStyle { start, before, after } => RichEdit::SetStyle {
                start: *start,
                before: after.clone(),
                after: before.clone(),
            },
        }
    }
}

/// An embedded image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub name: String,
    pub bytes: Vec<u8>,
    pub content_type: String,
}

const IMAGE_BASE: u32 = 0xE000;
/// Last private-use code point we hand out (end of the BMP private-use area).
const IMAGE_MAX: u32 = 0xF8FF;

/// Table of images; each is referenced in text by a placeholder char `U+E000 + index`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImageTable {
    images: Vec<Image>,
}

impl ImageTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an image and returns its placeholder char, or `None` if the table is full.
    pub fn add_image(&mut self, name: &str, bytes: Vec<u8>, content_type: &str) -> Option<char> {
        let code = IMAGE_BASE + self.images.len() as u32;
        if code > IMAGE_MAX {
            return None;
        }
        let ch = char::from_u32(code)?;
        self.images.push(Image {
            name: name.to_string(),
            bytes,
            content_type: content_type.to_string(),
        });
        Some(ch)
    }

    /// Index of a placeholder char (whether or not the image exists).
    pub fn index_of(c: char) -> Option<usize> {
        let code = c as u32;
        (IMAGE_BASE..=IMAGE_MAX).contains(&code).then(|| (code - IMAGE_BASE) as usize)
    }

    /// The image a placeholder char refers to.
    pub fn get(&self, c: char) -> Option<&Image> {
        Self::index_of(c).and_then(|i| self.images.get(i))
    }

    /// True if `c` is in the placeholder range but has no image.
    pub fn is_unknown(&self, c: char) -> bool {
        Self::index_of(c).is_some() && self.get(c).is_none()
    }

    /// Placeholder chars in `text` that have no image.
    pub fn unknown_in(&self, text: &str) -> Vec<char> {
        text.chars().filter(|&c| self.is_unknown(c)).collect()
    }

    pub fn len(&self) -> usize {
        self.images.len()
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty()
    }

    pub fn images(&self) -> &[Image] {
        &self.images
    }
}

/// Per-line style vectors parallel to the buffer's chars, plus the image table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rich {
    lines: Vec<Vec<Style>>,
    /// Style given to newly inserted text.
    pub pending: Style,
    pub images: ImageTable,
}

impl Default for Rich {
    fn default() -> Self {
        Self::new()
    }
}

fn advance(pos: Pos, text: &str) -> Pos {
    let mut p = pos;
    for c in text.chars() {
        if c == '\n' {
            p = (p.0 + 1, 0);
        } else {
            p.1 += 1;
        }
    }
    p
}

impl Rich {
    /// Empty document: one empty line.
    pub fn new() -> Self {
        Rich { lines: vec![Vec::new()], pending: Style::NONE, images: ImageTable::new() }
    }

    /// Unstyled document matching `text` (split on `'\n'`).
    pub fn from_text(text: &str) -> Self {
        let mut r = Self::new();
        r.lines = text.split('\n').map(|l| vec![Style::NONE; l.chars().count()]).collect();
        r
    }

    /// Per-line style vectors.
    pub fn lines(&self) -> &[Vec<Style>] {
        &self.lines
    }

    /// Replaces the styles of a line (used by readers). Ignored if out of range.
    pub fn set_line_styles(&mut self, line: usize, styles: Vec<Style>) {
        if let Some(l) = self.lines.get_mut(line) {
            *l = styles;
        }
    }

    pub fn style_at(&self, pos: Pos) -> Option<Style> {
        self.lines.get(pos.0).and_then(|l| l.get(pos.1)).copied()
    }

    fn clamp(&self, pos: Pos) -> Pos {
        let line = pos.0.min(self.lines.len() - 1);
        (line, pos.1.min(self.lines[line].len()))
    }

    /// Mirrors `edit`. Inserted text gets `self.pending` (newlines get none).
    /// Returns the resolved record to store for undo/redo.
    pub fn apply(&mut self, edit: &Edit) -> RichEdit {
        match edit {
            Edit::Insert { pos, text } => {
                let styles = text
                    .chars()
                    .map(|c| if c == '\n' { Style::NONE } else { self.pending })
                    .collect();
                let re = RichEdit::Insert { pos: self.clamp(*pos), text: text.clone(), styles };
                self.apply_rich(&re);
                re
            }
            Edit::Delete { pos, text } => {
                let pos = self.clamp(*pos);
                let styles = self.get_flat(pos, advance(pos, text));
                let re = RichEdit::Delete { pos, text: text.clone(), styles };
                self.apply_rich(&re);
                re
            }
        }
    }

    /// Applies a resolved record exactly (used for undo/redo).
    pub fn apply_rich(&mut self, re: &RichEdit) {
        match re {
            RichEdit::Insert { pos, text, styles } => self.insert(self.clamp(*pos), text, styles),
            RichEdit::Delete { pos, text, .. } => {
                let pos = self.clamp(*pos);
                self.delete(pos, advance(pos, text));
            }
            RichEdit::SetStyle { start, after, .. } => self.set_flat(self.clamp(*start), after),
        }
    }

    /// Sets (`on`) or clears `flag` over `[start, end)` and returns the record.
    /// Returns `None` if the range is empty.
    pub fn set_range(&mut self, start: Pos, end: Pos, flag: Style, on: bool) -> Option<RichEdit> {
        let (start, end) = self.ordered(start, end);
        let before = self.get_flat(start, end);
        if before.is_empty() {
            return None;
        }
        let after = self.recompute(start, end, &before, |s| if on { s.with(flag) } else { s.without(flag) });
        self.finish_style(start, before, after)
    }

    /// Toggles `flag` over the range: cleared if every char has it, otherwise set.
    pub fn toggle_range(&mut self, start: Pos, end: Pos, flag: Style) -> Option<RichEdit> {
        let (start, end) = self.ordered(start, end);
        let text_styles = self.text_styles(start, end);
        let all = !text_styles.is_empty() && text_styles.iter().all(|s| s.contains(flag));
        self.set_range(start, end, flag, !all)
    }

    fn finish_style(&mut self, start: Pos, before: Vec<Style>, after: Vec<Style>) -> Option<RichEdit> {
        if before == after {
            return None;
        }
        let re = RichEdit::SetStyle { start, before, after };
        self.apply_rich(&re);
        Some(re)
    }

    fn ordered(&self, a: Pos, b: Pos) -> (Pos, Pos) {
        let (a, b) = (self.clamp(a), self.clamp(b));
        if a <= b { (a, b) } else { (b, a) }
    }

    /// Styles of real chars only (no newline slots) in the range.
    fn text_styles(&self, start: Pos, end: Pos) -> Vec<Style> {
        let mut out = Vec::new();
        for l in start.0..=end.0 {
            let Some(line) = self.lines.get(l) else { break };
            let from = if l == start.0 { start.1 } else { 0 };
            let to = if l == end.0 { end.1 } else { line.len() };
            if from < to {
                out.extend_from_slice(&line[from..to.min(line.len())]);
            }
        }
        out
    }

    fn recompute(&self, start: Pos, end: Pos, before: &[Style], f: impl Fn(Style) -> Style) -> Vec<Style> {
        // Newline slots (the end of each line crossed) stay NONE.
        let mut out = Vec::with_capacity(before.len());
        let mut idx = 0;
        for l in start.0..=end.0 {
            let len = self.lines.get(l).map_or(0, Vec::len);
            let from = if l == start.0 { start.1 } else { 0 };
            let to = if l == end.0 { end.1 } else { len };
            for _ in from..to {
                out.push(f(before.get(idx).copied().unwrap_or_default()));
                idx += 1;
            }
            if l != end.0 {
                out.push(Style::NONE);
                idx += 1;
            }
        }
        out
    }

    /// Flat styles over `[start, end)`; each crossed line break contributes `NONE`.
    fn get_flat(&self, start: Pos, end: Pos) -> Vec<Style> {
        let mut out = Vec::new();
        for l in start.0..=end.0 {
            let Some(line) = self.lines.get(l) else { break };
            let from = if l == start.0 { start.1 } else { 0 };
            let to = if l == end.0 { end.1 } else { line.len() };
            let to = to.min(line.len());
            if from < to {
                out.extend_from_slice(&line[from..to]);
            }
            if l != end.0 {
                out.push(Style::NONE);
            }
        }
        out
    }

    /// Writes flat styles starting at `start` (line-break slots are skipped).
    fn set_flat(&mut self, start: Pos, styles: &[Style]) {
        let (mut l, mut c) = start;
        for &s in styles {
            let Some(line) = self.lines.get_mut(l) else { return };
            if c < line.len() {
                line[c] = s;
                c += 1;
            } else {
                l += 1;
                c = 0;
            }
        }
    }

    fn insert(&mut self, pos: Pos, text: &str, styles: &[Style]) {
        let tail = self.lines[pos.0].split_off(pos.1);
        let mut cur = pos.0;
        for (i, ch) in text.chars().enumerate() {
            if ch == '\n' {
                cur += 1;
                self.lines.insert(cur, Vec::new());
            } else {
                let s = styles.get(i).copied().unwrap_or(self.pending);
                self.lines[cur].push(s);
            }
        }
        self.lines[cur].extend(tail);
    }

    fn delete(&mut self, start: Pos, end: Pos) {
        let end = self.clamp(end);
        if end <= start {
            return;
        }
        let tail = self.lines[end.0].split_off(end.1);
        self.lines[start.0].truncate(start.1);
        self.lines[start.0].extend(tail);
        self.lines.drain(start.0 + 1..=end.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;

    const B: Style = Style::BOLD;
    const I: Style = Style::ITALIC;
    const N: Style = Style::NONE;

    fn ins(pos: Pos, t: &str) -> Edit {
        Edit::Insert { pos, text: t.into() }
    }
    fn del(pos: Pos, t: &str) -> Edit {
        Edit::Delete { pos, text: t.into() }
    }

    /// Applies `e` to a buffer and rich doc, checking alignment.
    fn aligned(b: &Buffer, r: &Rich) {
        assert_eq!(b.lines().len(), r.lines().len());
        for (l, s) in b.lines().iter().zip(r.lines()) {
            assert_eq!(l.chars().count(), s.len());
        }
    }

    #[test]
    fn insert_takes_pending_style() {
        let mut r = Rich::from_text("ab");
        r.pending = B;
        r.apply(&ins((0, 1), "xy"));
        assert_eq!(r.lines()[0], vec![N, B, B, N]);
    }

    #[test]
    fn multiline_insert_and_join() {
        let mut b = Buffer::from_text("héllo wörld");
        let mut r = Rich::from_text("héllo wörld");
        r.pending = I;
        let e = ins((0, 5), "é\nñ");
        b.apply(&e);
        let rec = r.apply(&e);
        aligned(&b, &r);
        assert_eq!(r.lines().len(), 2);
        assert_eq!(r.lines()[0].len(), 6);
        assert_eq!(r.lines()[1], vec![I, N, N, N, N, N, N]);
        // join the lines again by deleting the newline
        let d = del((0, 6), "\n");
        b.apply(&d);
        r.apply(&d);
        aligned(&b, &r);
        assert_eq!(r.lines().len(), 1);
        let _ = rec;
    }

    #[test]
    fn delete_undo_redo_restores_styles() {
        let mut r = Rich::from_text("one\ntwo\nthree");
        r.set_range((0, 1), (2, 2), B, true);
        let before = r.clone();
        let rec = r.apply(&del((0, 1), "ne\ntwo\nth"));
        assert_eq!(r.lines().len(), 1);
        assert_eq!(r.lines()[0].len(), 4);
        let after = r.clone();
        r.apply_rich(&rec.inverse());
        assert_eq!(r, before);
        r.apply_rich(&rec);
        assert_eq!(r, after);
    }

    #[test]
    fn insert_undo_redo() {
        let mut r = Rich::from_text("ab");
        r.pending = B;
        let before = r.clone();
        let rec = r.apply(&ins((0, 1), "x\ny"));
        let after = r.clone();
        r.apply_rich(&rec.inverse());
        assert_eq!(r, before);
        r.apply_rich(&rec);
        assert_eq!(r, after);
    }

    #[test]
    fn toggle_and_undo() {
        let mut r = Rich::from_text("héy\nyo");
        let rec = r.toggle_range((0, 1), (1, 1), B).expect("changed");
        assert_eq!(r.lines()[0], vec![N, B, B]);
        assert_eq!(r.lines()[1], vec![B, N]);
        r.apply_rich(&rec.inverse());
        assert_eq!(r, Rich::from_text("héy\nyo"));
        r.apply_rich(&rec);
        assert_eq!(r.lines()[1], vec![B, N]);
        // toggling again clears
        r.toggle_range((0, 1), (1, 1), B);
        assert!(r.lines().iter().flatten().all(|s| s.is_empty()));
        // empty range
        assert!(r.toggle_range((0, 1), (0, 1), B).is_none());
    }

    #[test]
    fn out_of_range_is_clamped() {
        let mut r = Rich::from_text("ab");
        r.apply(&ins((9, 9), "c"));
        assert_eq!(r.lines()[0].len(), 3);
        r.apply(&del((5, 0), "zzzz"));
        assert_eq!(r.lines().len(), 1);
    }

    #[test]
    fn images() {
        let mut r = Rich::new();
        let c = r.images.add_image("a.png", vec![1, 2], "image/png").expect("room");
        assert_eq!(c, '\u{E000}');
        let d = r.images.add_image("b.jpg", vec![3], "image/jpeg").expect("room");
        assert_eq!(d, '\u{E001}');
        assert_eq!(r.images.get(d).map(|i| i.name.as_str()), Some("b.jpg"));
        assert!(r.images.get('a').is_none());
        assert!(!r.images.is_unknown('a'));
        assert!(r.images.is_unknown('\u{E005}'));
        assert_eq!(r.images.unknown_in("x\u{E000}\u{E007}"), vec!['\u{E007}']);
    }

    #[test]
    fn cut_paste_references_same_image() {
        let mut r = Rich::new();
        let c = r.images.add_image("a.png", vec![9], "image/png").expect("room");
        let text = format!("a{c}b");
        let mut b = Buffer::from_text(&text);
        r.apply(&ins((0, 0), &text));
        let d = del((0, 1), &c.to_string());
        b.apply(&d);
        r.apply(&d);
        let e = ins((0, 2), &c.to_string());
        b.apply(&e);
        r.apply(&e);
        aligned(&b, &r);
        assert_eq!(b.to_text(), format!("ab{c}"));
        assert_eq!(r.images.get(c).map(|i| i.bytes.clone()), Some(vec![9]));
        assert_eq!(r.images.len(), 1);
    }
}
