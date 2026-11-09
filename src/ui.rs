//! Rendering of the editor to a virtual [`Screen`], and diff drawing to the terminal.

use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    queue,
    style::{Attribute, Print, SetAttribute},
};
use unicode_width::UnicodeWidthChar;

use crate::editor::{Editor, CHROME_ROWS};

/// Display width of a tab stop.
pub const TAB_WIDTH: usize = 8;

/// Kind of highlighted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Highlight {
    Selection,
    Match,
}

/// Width of the line-number gutter (digits plus a separating space), 0 when off.
fn gutter_width(ed: &Editor) -> usize {
    if ed.line_numbers() {
        ed.buffer().line_count().max(1).to_string().len() + 1
    } else {
        0
    }
}

/// A rendered frame: one string per terminal row, each exactly `width` columns wide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub rows: Vec<String>,
    /// Rows drawn in reverse video.
    pub inverse: Vec<bool>,
    /// Cursor `(column, row)`.
    pub cursor: (usize, usize),
    /// Per row: highlighted display-column ranges `(start, end, kind)`, end exclusive.
    pub highlights: Vec<Vec<(usize, usize, Highlight)>>,
    pub width: usize,
    pub height: usize,
}

fn cell_width(c: char) -> usize {
    c.width().unwrap_or(1)
}

/// Display width of `c` when it starts at display column `col`.
fn char_cols(c: char, col: usize, tab: usize) -> usize {
    if c == '\t' {
        tab - col % tab
    } else {
        cell_width(c)
    }
}

/// Truncates or pads `s` with spaces to exactly `width` display columns.
fn fit(s: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = cell_width(c);
        if used + w > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push_str(&" ".repeat(width - used));
    out
}

fn center(s: &str, width: usize) -> String {
    let w: usize = s.chars().map(cell_width).sum();
    if w >= width {
        return fit(s, width);
    }
    let left = (width - w) / 2;
    fit(&format!("{}{}", " ".repeat(left), s), width)
}

/// Display column of character index `idx` in `line`.
fn display_col(line: &str, idx: usize, tab: usize) -> usize {
    let mut col = 0;
    for c in line.chars().take(idx) {
        col += char_cols(c, col, tab);
    }
    col
}

/// The visible slice `[left, left+cols)` of `line`, tabs expanded, padded to `cols`.
fn visible_slice(line: &str, left: usize, cols: usize, tab: usize) -> String {
    let right = left + cols;
    let mut out = String::new();
    let mut used = 0;
    let mut col = 0;
    for c in line.chars() {
        if col >= right {
            break;
        }
        let w = char_cols(c, col, tab);
        if w == 0 {
            if used > 0 {
                out.push(c);
            }
            continue;
        }
        let start = col;
        col += w;
        if col <= left {
            continue;
        }
        let vis_start = start.max(left);
        let vis_end = col.min(right);
        let vis = vis_end - vis_start;
        if c == '\t' || vis < w {
            out.push_str(&" ".repeat(vis));
        } else if c.is_control() {
            out.push('?');
        } else {
            out.push(c);
        }
        used += vis;
    }
    out.push_str(&" ".repeat(cols.saturating_sub(used)));
    out
}

/// Sizes the editor viewport for a `width` x `height` terminal and scrolls so the
/// cursor is visible. `left_col` is interpreted as a display column here.
pub fn prepare(ed: &mut Editor, width: usize, height: usize) {
    let rows = height.saturating_sub(CHROME_ROWS).max(1);
    let cols = width.saturating_sub(gutter_width(ed)).max(1);
    ed.set_viewport(cols, rows);
    let (l, c) = ed.buffer().cursor();
    let dc = display_col(ed.buffer().line(l).unwrap_or(""), c, ed.tab_size());
    if l < ed.top_line {
        ed.top_line = l;
    } else if l >= ed.top_line + rows {
        ed.top_line = l + 1 - rows;
    }
    if dc < ed.left_col {
        ed.left_col = dc;
    } else if dc >= ed.left_col + cols {
        ed.left_col = dc + 1 - cols;
    }
}

fn title_text(ed: &Editor) -> String {
    let mut s = format!(
        "NanoRust {}   {}",
        env!("CARGO_PKG_VERSION"),
        ed.title_name()
    );
    if ed.modified() {
        s.push_str("   Modified");
    }
    s
}

const SHORTCUTS: [[(&str, &str); 6]; 2] = [
    [
        ("^G", "Help"),
        ("^O", "Write Out"),
        ("^W", "Where Is"),
        ("^K", "Cut"),
        ("^C", "Location"),
        ("^A", "Home"),
    ],
    [
        ("^X", "Exit"),
        ("^R", "Read File"),
        ("^\\", "Replace"),
        ("^U", "Paste"),
        ("^_", "Go To Line"),
        ("^E", "End"),
    ],
];

fn shortcut_row(row: usize, width: usize, prompt: bool) -> String {
    if prompt {
        return if row == 0 {
            fit("^C Cancel", width)
        } else {
            fit("", width)
        };
    }
    let items = &SHORTCUTS[row];
    let cell = (width / items.len()).max(1);
    let mut s = String::new();
    for (k, d) in items {
        s.push_str(&fit(&format!("{k} {d}"), cell));
    }
    fit(&s, width)
}

/// Renders the editor to a virtual screen of `width` x `height`. Never panics,
/// whatever the size. Call [`prepare`] first so the scroll offsets are current.
pub fn render(ed: &Editor, width: usize, height: usize) -> Screen {
    let text_rows = height.saturating_sub(CHROME_ROWS);
    let mut rows: Vec<String> = Vec::with_capacity(height);
    let mut inverse = Vec::with_capacity(height);
    let mut cursor = (0, 0);
    let mut highlights: Vec<Vec<(usize, usize, Highlight)>> = Vec::with_capacity(height);

    if ed.help_visible() {
        return help_screen(width, height);
    }

    // Chrome pieces.
    let title = fit(&center(&title_text(ed), width), width);
    let (status, status_cursor) = status_line(ed, width);

    if height >= CHROME_ROWS {
        rows.push(title);
        inverse.push(true);
        highlights.push(Vec::new());
        let (l, c) = ed.buffer().cursor();
        let tab = ed.tab_size();
        let gutter = gutter_width(ed).min(width);
        let tw = width - gutter;
        let sel = ed.selection_range();
        let mat = ed.match_range();
        for r in 0..text_rows {
            let li = ed.top_line + r;
            let line = ed.buffer().line(li).unwrap_or("");
            let empty = li >= ed.buffer().line_count();
            let mut row = String::new();
            if gutter > 0 {
                let num = if empty {
                    String::new()
                } else {
                    format!("{:>w$} ", li + 1, w = gutter - 1)
                };
                row.push_str(&fit(&num, gutter));
            }
            let mut hl = Vec::new();
            if empty {
                row.push_str(&" ".repeat(tw));
            } else {
                row.push_str(&visible_slice(line, ed.left_col, tw, tab));
                for (range, kind) in [(sel, Highlight::Selection), (mat, Highlight::Match)] {
                    if let Some(((al, ac), (bl, bc))) = range {
                        if li < al || li > bl {
                            continue;
                        }
                        let n = line.chars().count();
                        let s = if li == al { ac } else { 0 };
                        let e = if li == bl { bc } else { n };
                        let ds = display_col(line, s.min(n), tab);
                        let de = display_col(line, e.min(n), tab);
                        let a = ds.saturating_sub(ed.left_col).min(tw);
                        let b = de.saturating_sub(ed.left_col).min(tw);
                        if a < b {
                            hl.push((a + gutter, b + gutter, kind));
                        }
                    }
                }
            }
            rows.push(row);
            inverse.push(false);
            highlights.push(hl);
        }
        let dc = display_col(ed.buffer().line(l).unwrap_or(""), c, tab);
        let cur_row = 1 + l.saturating_sub(ed.top_line).min(text_rows.saturating_sub(1));
        let cur_col = (gutter + dc.saturating_sub(ed.left_col)).min(width.saturating_sub(1));
        cursor = (cur_col, cur_row);
        let status_row = rows.len();
        rows.push(status);
        inverse.push(false);
        highlights.push(Vec::new());
        if let Some(cc) = status_cursor {
            cursor = (cc, status_row);
        }
        for i in 0..2 {
            rows.push(shortcut_row(i, width, ed.prompt_kind().is_some()));
            inverse.push(false);
            highlights.push(Vec::new());
        }
    } else if height > 0 {
        // Too small for the text area: keep the status line, then the title, then the bar.
        let mut parts = vec![(status, false)];
        parts.push((title, true));
        parts.push((shortcut_row(0, width, ed.prompt_kind().is_some()), false));
        let mut chosen: Vec<(String, bool)> = parts.into_iter().take(height).collect();
        // Display order: title, status, bar.
        chosen.sort_by_key(|(_, inv)| !*inv);
        let has_title = chosen.iter().any(|(_, i)| *i);
        let status_row = if has_title { 1 } else { 0 };
        if let Some(cc) = status_cursor {
            cursor = (cc, status_row.min(height - 1));
        }
        for (s, i) in chosen {
            rows.push(s);
            inverse.push(i);
            highlights.push(Vec::new());
        }
    }
    cursor.0 = cursor.0.min(width.saturating_sub(1));
    Screen {
        rows,
        inverse,
        cursor,
        highlights,
        width,
        height,
    }
}

const HELP_KEYS: [(&str, &str); 22] = [
    ("^G", "Display this help"),
    ("^X", "Exit the editor"),
    ("^O", "Write the buffer to a file"),
    ("^R", "Insert a file into the buffer"),
    ("^W", "Search for text"),
    ("^\\", "Search and replace"),
    ("^_", "Go to a line number"),
    ("^K", "Cut the line or selection"),
    ("^U", "Paste the cut text"),
    ("^6", "Set or clear the selection mark"),
    ("^C", "Show cursor location / cancel"),
    ("^A / Home", "Start of line"),
    ("^E / End", "End of line"),
    ("^Y / PgUp", "Previous page"),
    ("^V / PgDn", "Next page"),
    ("Arrows", "Move the cursor"),
    ("Tab", "Insert a tab (or spaces with -E)"),
    ("Backspace", "Delete before the cursor"),
    ("Delete", "Delete at the cursor"),
    ("M-U", "Undo"),
    ("M-E", "Redo"),
    ("Any key", "Close this help"),
];

/// The full-screen help: title, key table, clipped to the screen size.
fn help_screen(width: usize, height: usize) -> Screen {
    let mut lines = vec![center("NanoRust Help", width), " ".repeat(width)];
    for (k, d) in HELP_KEYS {
        lines.push(fit(&format!(" {k:<12} {d}"), width));
    }
    lines.resize(height, " ".repeat(width));
    lines.truncate(height);
    let inverse = (0..height).map(|i| i == 0).collect();
    Screen {
        rows: lines,
        inverse,
        cursor: (0, 0),
        highlights: vec![Vec::new(); height],
        width,
        height,
    }
}

/// The status line and, when a prompt with input is open, the cursor column.
fn status_line(ed: &Editor, width: usize) -> (String, Option<usize>) {
    if let (Some(label), Some(text)) = (ed.prompt_label(), ed.prompt_text()) {
        if ed.prompt_kind() == Some(crate::editor::PromptKind::SaveModified) {
            return (fit(&label, width), None);
        }
        let prefix = format!("{label}: ");
        let full = format!("{prefix}{text}");
        let total: usize = full.chars().map(cell_width).sum();
        // Keep the end of the input visible.
        let skip = total.saturating_sub(width.saturating_sub(1));
        let mut shown = String::new();
        let mut skipped = 0;
        for c in full.chars() {
            if skipped < skip {
                skipped += cell_width(c);
            } else {
                shown.push(c);
            }
        }
        let cur = total - skipped.min(total);
        return (fit(&shown, width), Some(cur));
    }
    match ed.status() {
        Some(m) => (center(&format!("[ {m} ]"), width), None),
        None => (" ".repeat(width), None),
    }
}

/// Draws `screen` to `out`, writing only rows that differ from `prev` (all rows when
/// `prev` is `None` or has a different size), then places the cursor.
pub fn draw<W: Write>(out: &mut W, screen: &Screen, prev: Option<&Screen>) -> io::Result<()> {
    let same_size = prev.is_some_and(|p| p.width == screen.width && p.height == screen.height);
    queue!(out, Hide)?;
    if !same_size {
        queue!(
            out,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::All)
        )?;
    }
    for (i, row) in screen.rows.iter().enumerate() {
        let inv = screen.inverse.get(i).copied().unwrap_or(false);
        if let (true, Some(p)) = (same_size, prev) {
            if p.rows.get(i) == Some(row) && p.inverse.get(i).copied().unwrap_or(false) == inv {
                continue;
            }
        }
        queue!(out, MoveTo(0, i as u16))?;
        if inv {
            queue!(out, SetAttribute(Attribute::Reverse))?;
        }
        let hl = screen.highlights.get(i).map(Vec::as_slice).unwrap_or(&[]);
        if hl.is_empty() {
            queue!(out, Print(row))?;
        } else {
            draw_highlighted(out, row, hl)?;
        }
        if inv {
            queue!(out, SetAttribute(Attribute::Reset))?;
        }
    }
    queue!(
        out,
        MoveTo(screen.cursor.0 as u16, screen.cursor.1 as u16),
        Show
    )?;
    out.flush()
}

/// Prints `row`, drawing the highlighted display-column ranges in reverse video
/// (selection) or bold reverse (match).
fn draw_highlighted<W: Write>(
    out: &mut W,
    row: &str,
    hl: &[(usize, usize, Highlight)],
) -> io::Result<()> {
    let mut col = 0;
    let mut cur: Option<Highlight> = None;
    for c in row.chars() {
        let want = hl
            .iter()
            .rev()
            .find(|(a, b, _)| col >= *a && col < *b)
            .map(|h| h.2);
        if want != cur {
            queue!(out, SetAttribute(Attribute::Reset))?;
            match want {
                Some(Highlight::Selection) => queue!(out, SetAttribute(Attribute::Reverse))?,
                Some(Highlight::Match) => {
                    queue!(out, SetAttribute(Attribute::Reverse), SetAttribute(Attribute::Bold))?
                }
                None => {}
            }
            cur = want;
        }
        queue!(out, Print(c))?;
        col += cell_width(c);
    }
    if cur.is_some() {
        queue!(out, SetAttribute(Attribute::Reset))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::Action;

    fn ed_with(text: &str) -> Editor {
        let mut ed = Editor::new();
        for c in text.chars() {
            ed.handle(if c == '\n' {
                Action::Newline
            } else if c == '\t' {
                Action::Tab
            } else {
                Action::Insert(c)
            });
        }
        ed
    }

    fn scr(ed: &mut Editor, w: usize, h: usize) -> Screen {
        prepare(ed, w, h);
        render(ed, w, h)
    }

    #[test]
    fn layout_and_title() {
        let mut ed = ed_with("hi");
        let s = scr(&mut ed, 80, 10);
        assert_eq!(s.rows.len(), 10);
        assert!(s.rows.iter().all(|r| r.chars().count() == 80));
        assert!(s.rows[0].contains("NanoRust"));
        assert!(s.rows[0].contains("New Buffer"));
        assert!(s.rows[0].contains("Modified"));
        assert!(s.rows[1].starts_with("hi"));
        assert!(s.rows[9].contains("^X Exit"));
        assert!(s.rows[8].contains("^G Help"));
        assert!(s.rows[8].contains("^O Write Out"));
        assert_eq!(s.cursor, (2, 1));
        assert!(s.inverse[0]);
    }

    #[test]
    fn unmodified_has_no_indicator() {
        let mut ed = Editor::new();
        let s = scr(&mut ed, 40, 10);
        assert!(!s.rows[0].contains("Modified"));
    }

    #[test]
    fn vertical_scroll() {
        let text: String = (0..20).map(|i| format!("line{i}\n")).collect();
        let mut ed = ed_with(&text);
        let s = scr(&mut ed, 30, 8); // 4 text rows
        assert_eq!(ed.top_line, 17);
        assert!(s.rows[1].starts_with("line17"));
        assert!(s.rows[4].starts_with(" "));
        assert_eq!(s.cursor, (0, 4));
        for _ in 0..20 {
            ed.handle(Action::Up);
        }
        let s = scr(&mut ed, 30, 8);
        assert_eq!(ed.top_line, 0);
        assert!(s.rows[1].starts_with("line0"));
        assert_eq!(s.cursor, (0, 1));
    }

    #[test]
    fn horizontal_scroll() {
        let mut ed = ed_with(&"abcdefghij".repeat(3));
        let s = scr(&mut ed, 12, 8);
        assert_eq!(ed.left_col, 30 + 1 - 12);
        assert_eq!(s.cursor.0, 11);
        assert_eq!(s.rows[1], format!("{:<12}", "jabcdefghij").replace(' ', " "));
        ed.handle(Action::Home);
        let s = scr(&mut ed, 12, 8);
        assert_eq!(ed.left_col, 0);
        assert!(s.rows[1].starts_with("abcdefghijab"));
        assert_eq!(s.cursor, (0, 1));
    }

    #[test]
    fn tabs_expand() {
        let mut ed = ed_with("a\tb");
        let s = scr(&mut ed, 30, 8);
        assert_eq!(&s.rows[1][..10], "a       b ");
        assert_eq!(s.cursor, (9, 1));
    }

    #[test]
    fn wide_chars() {
        let mut ed = ed_with("日本語x");
        let s = scr(&mut ed, 30, 8);
        assert!(s.rows[1].starts_with("日本語x"));
        assert_eq!(s.cursor, (7, 1));
        assert_eq!(s.rows[1].chars().map(cell_width).sum::<usize>(), 30);
        // Narrow terminal: wide char cut at the edge is blanked, never overflows.
        let s = scr(&mut ed, 5, 8);
        assert!(s.rows.iter().all(|r| r.chars().map(cell_width).sum::<usize>() == 5));
        assert!(s.cursor.0 < 5);
    }

    #[test]
    fn slice_cuts_wide_char_at_left_edge() {
        assert_eq!(visible_slice("日本", 1, 4, 8), " 本 ");
        assert_eq!(visible_slice("a日", 0, 2, 8), "a ");
    }

    #[test]
    fn status_message() {
        let mut ed = Editor::new();
        ed.set_status("Read 3 lines");
        let s = scr(&mut ed, 40, 10);
        assert!(s.rows[7].contains("[ Read 3 lines ]"));
    }

    #[test]
    fn write_out_prompt() {
        let mut ed = ed_with("x");
        ed.handle(Action::WriteOut);
        for c in "ab".chars() {
            ed.handle(Action::Insert(c));
        }
        let s = scr(&mut ed, 60, 10);
        let row = &s.rows[7];
        assert!(row.starts_with("File Name to Write: "));
        assert!(row.trim_end().ends_with("ab"));
        assert_eq!(s.cursor.1, 7);
        assert_eq!(s.cursor.0, row.trim_end().chars().count());
        assert!(s.rows[8].contains("^C Cancel"));
    }

    #[test]
    fn save_modified_prompt() {
        let mut ed = ed_with("x");
        ed.handle(Action::Exit);
        let s = scr(&mut ed, 60, 10);
        assert!(s.rows[7].starts_with("Save modified buffer?"));
    }

    #[test]
    fn long_prompt_keeps_cursor_in_view() {
        let mut ed = ed_with("x");
        ed.handle(Action::WriteOut);
        for _ in 0..50 {
            ed.handle(Action::Insert('z'));
        }
        let s = scr(&mut ed, 20, 10);
        assert!(s.cursor.0 < 20);
        assert!(s.rows[7].trim_end().ends_with('z'));
    }

    #[test]
    fn tiny_sizes_do_not_panic() {
        let mut ed = ed_with("日本\tabc\ndef");
        ed.set_status("hello");
        for w in 0..8 {
            for h in 0..8 {
                let s = scr(&mut ed, w, h);
                assert_eq!(s.rows.len(), h.min(h));
                for r in &s.rows {
                    assert_eq!(r.chars().map(cell_width).sum::<usize>(), w);
                }
                assert!(s.cursor.0 <= w.max(1) && s.cursor.1 <= h.max(1));
            }
        }
        ed.handle(Action::WriteOut);
        for w in 0..8 {
            for h in 0..8 {
                scr(&mut ed, w, h);
            }
        }
    }

    #[test]
    fn draw_diffs_rows() {
        let mut ed = ed_with("a");
        let s1 = scr(&mut ed, 20, 8);
        let mut full = Vec::new();
        draw(&mut full, &s1, None).unwrap();
        ed.handle(Action::Insert('b'));
        let s2 = scr(&mut ed, 20, 8);
        let mut diff = Vec::new();
        draw(&mut diff, &s2, Some(&s1)).unwrap();
        assert!(diff.len() < full.len());
        let d = String::from_utf8_lossy(&diff);
        assert!(d.contains("ab"));
        let mut same = Vec::new();
        draw(&mut same, &s2, Some(&s2)).unwrap();
        assert!(same.len() < diff.len());
    }

    #[test]
    fn gutter_and_cursor() {
        let text: String = (1..=12).map(|i| format!("l{i}\n")).collect();
        let mut ed = ed_with(&text);
        ed.configure(None, false, true);
        ed.handle(Action::Up);
        ed.handle(Action::Up);
        let s = scr(&mut ed, 20, 8);
        // 13 lines -> 2 digits + space
        let n = ed.top_line + 1;
        assert!(s.rows[1].starts_with(&format!("{n:>2} l{n}")), "{:?}", s.rows[1]);
        assert!(s.rows.iter().all(|r| r.chars().count() == 20));
        let cur_line = ed.buffer().cursor().0;
        assert_eq!(s.cursor, (3, 1 + cur_line - ed.top_line));
        ed.handle(Action::End);
        let s = scr(&mut ed, 20, 8);
        assert_eq!(s.cursor.0, 3 + ed.buffer().line(cur_line).unwrap().chars().count());
    }

    #[test]
    fn gutter_off_by_default() {
        let mut ed = ed_with("x");
        let s = scr(&mut ed, 20, 8);
        assert!(s.rows[1].starts_with('x'));
    }

    #[test]
    fn gutter_horizontal_scroll() {
        let mut ed = ed_with(&"a".repeat(30));
        ed.configure(None, false, true);
        let s = scr(&mut ed, 12, 8);
        assert_eq!(s.cursor.0, 11);
        assert!(s.rows[1].starts_with("1 "));
    }

    #[test]
    fn custom_tab_size() {
        let mut ed = ed_with("a\tb");
        ed.configure(Some(4), false, false);
        let s = scr(&mut ed, 30, 8);
        assert_eq!(&s.rows[1][..6], "a   b ");
        assert_eq!(s.cursor, (5, 1));
    }

    #[test]
    fn tabs_to_spaces_inserts_to_next_stop() {
        let mut ed = Editor::new();
        ed.configure(Some(4), true, false);
        ed.handle(Action::Insert('a'));
        ed.handle(Action::Tab);
        assert_eq!(ed.buffer().line(0), Some("a   "));
        ed.handle(Action::Tab);
        assert_eq!(ed.buffer().line(0), Some("a       "));
        let mut ed = Editor::new();
        ed.configure(Some(4), false, false);
        ed.handle(Action::Tab);
        assert_eq!(ed.buffer().line(0), Some("\t"));
    }

    #[test]
    fn selection_highlight() {
        let mut ed = ed_with("hello world");
        ed.handle(Action::Home);
        ed.set_mark_for_test();
        for _ in 0..5 {
            ed.handle(Action::Right);
        }
        let s = scr(&mut ed, 30, 8);
        assert_eq!(s.highlights[1], vec![(0, 5, Highlight::Selection)]);
        assert!(s.highlights[2].is_empty());
    }

    #[test]
    fn match_highlight_with_gutter() {
        let mut ed = ed_with("hello world");
        ed.configure(None, false, true);
        ed.set_match_range(Some(((0, 6), (0, 11))));
        let s = scr(&mut ed, 30, 8);
        assert_eq!(s.highlights[1], vec![(8, 13, Highlight::Match)]);
        let mut out = Vec::new();
        draw(&mut out, &s, None).unwrap();
        assert!(String::from_utf8_lossy(&out).contains("world"));
    }

    #[test]
    fn help_screen_shown() {
        let mut ed = ed_with("secret text");
        ed.set_help_visible(true);
        let s = scr(&mut ed, 60, 30);
        assert_eq!(s.rows.len(), 30);
        assert!(s.rows[0].contains("Help"));
        assert!(s.rows.iter().any(|r| r.contains("^X") && r.contains("Exit")));
        assert!(!s.rows.iter().any(|r| r.contains("secret")));
        ed.handle(Action::Insert('q'));
        assert!(!ed.help_visible());
        assert_eq!(ed.buffer().line(0), Some("secret text"));
        let s = scr(&mut ed, 60, 30);
        assert!(s.rows[1].contains("secret"));
        ed.set_help_visible(true);
        for w in 0..6 {
            for h in 0..6 {
                scr(&mut ed, w, h);
            }
        }
    }
}
