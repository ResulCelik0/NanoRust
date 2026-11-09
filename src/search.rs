//! Text search and replace.
//!
//! Columns are char (not byte) columns. Matches never span lines; a query
//! containing `'\n'` or an empty query matches nothing.

use crate::buffer::{Buffer, Edit, Pos};
use crate::undo::History;

/// Search options.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub case_sensitive: bool,
    pub backward: bool,
    pub wrap: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            case_sensitive: true,
            backward: false,
            wrap: true,
        }
    }
}

/// A match: start position and length in chars.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: Pos,
    pub len: usize,
}

impl Match {
    pub fn end(&self) -> Pos {
        (self.start.0, self.start.1 + self.len)
    }
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Char columns where `needle` occurs in `line`, ascending.
fn line_matches(line: &str, needle: &[char], case: bool) -> Vec<usize> {
    let hay: Vec<char> = line.chars().collect();
    if needle.is_empty() || needle.len() > hay.len() {
        return Vec::new();
    }
    (0..=hay.len() - needle.len())
        .filter(|&i| {
            hay[i..i + needle.len()]
                .iter()
                .zip(needle)
                .all(|(a, b)| if case { a == b } else { fold(*a) == fold(*b) })
        })
        .collect()
}

fn needle_of(query: &str, case: bool) -> Option<Vec<char>> {
    if query.is_empty() || query.contains('\n') {
        return None;
    }
    let _ = case;
    Some(query.chars().collect())
}

fn all_matches(buf: &Buffer, query: &str, case: bool) -> Vec<Match> {
    let Some(needle) = needle_of(query, case) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (l, line) in buf.lines().iter().enumerate() {
        for c in line_matches(line, &needle, case) {
            out.push(Match {
                start: (l, c),
                len: needle.len(),
            });
        }
    }
    out
}

fn find_impl(
    buf: &Buffer,
    from: Pos,
    query: &str,
    opts: Options,
    inclusive: bool,
) -> Option<Match> {
    let all = all_matches(buf, query, opts.case_sensitive);
    if opts.backward {
        let before = all.iter().rev().find(|m| m.start < from);
        let pick = before.or_else(|| if opts.wrap { all.last() } else { None });
        pick.copied()
    } else {
        let after = all
            .iter()
            .find(|m| if inclusive { m.start >= from } else { m.start > from });
        let pick = after.or_else(|| if opts.wrap { all.first() } else { None });
        pick.copied()
    }
}

/// Finds the next match strictly after (forward) or strictly before (backward)
/// `from`, wrapping around if `opts.wrap`. A match at `from` itself is only found
/// by wrapping.
pub fn find(buf: &Buffer, from: Pos, query: &str, opts: Options) -> Option<Match> {
    find_impl(buf, from, query, opts, false)
}

/// Finds from the cursor and, on success, moves the cursor to the match start.
pub fn find_next(buf: &mut Buffer, query: &str, opts: Options) -> Option<Match> {
    let m = find(buf, buf.cursor(), query, opts)?;
    buf.clear_mark();
    buf.set_cursor(m.start);
    Some(m)
}

fn replace_edits(m: &Match, repl: &str) -> Vec<Edit> {
    let mut v = vec![Edit::Delete {
        pos: m.start,
        text: String::new(),
    }];
    if !repl.is_empty() {
        v.push(Edit::Insert {
            pos: m.start,
            text: repl.to_string(),
        });
    }
    v
}

/// Replaces `m` (already located in `buf`) and returns the applied edits.
fn do_replace(buf: &mut Buffer, m: &Match, repl: &str) -> Vec<Edit> {
    let text: String = buf
        .line(m.start.0)
        .unwrap_or("")
        .chars()
        .skip(m.start.1)
        .take(m.len)
        .collect();
    let mut edits = replace_edits(m, repl);
    if let Some(Edit::Delete { text: t, .. }) = edits.first_mut() {
        *t = text;
    }
    for e in &edits {
        buf.apply(e);
    }
    edits
}

/// Replaces the first match at or after the cursor (wrapping per `opts`; the
/// `backward` flag is ignored). Recorded as one undo step. Returns whether a
/// replacement happened; the cursor ends after the replacement.
pub fn replace_one(
    buf: &mut Buffer,
    hist: &mut History,
    query: &str,
    repl: &str,
    opts: Options,
) -> bool {
    let fwd = Options {
        backward: false,
        ..opts
    };
    let Some(m) = find_impl(buf, buf.cursor(), query, fwd, true) else {
        return false;
    };
    let edits = do_replace(buf, &m, repl);
    hist.record_group(edits);
    true
}

/// Replaces every match in the buffer as a single undo step; returns the count.
pub fn replace_all(
    buf: &mut Buffer,
    hist: &mut History,
    query: &str,
    repl: &str,
    case_sensitive: bool,
) -> usize {
    let matches = all_matches(buf, query, case_sensitive);
    let mut group = Vec::new();
    // Back to front so earlier positions stay valid.
    for m in matches.iter().rev() {
        group.extend(do_replace(buf, m, repl));
    }
    hist.record_group(group);
    matches.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> Options {
        Options::default()
    }

    #[test]
    fn forward_and_wrap() {
        let b = Buffer::from_text("foo bar\nbar foo");
        let m = find(&b, (0, 0), "foo", opts()).unwrap();
        assert_eq!(m.start, (1, 4));
        let m = find(&b, (1, 4), "foo", opts()).unwrap();
        assert_eq!(m.start, (0, 0));
        let no = Options { wrap: false, ..opts() };
        assert!(find(&b, (1, 4), "foo", no).is_none());
    }

    #[test]
    fn backward() {
        let b = Buffer::from_text("foo bar\nbar foo");
        let o = Options { backward: true, ..opts() };
        assert_eq!(find(&b, (1, 4), "foo", o).unwrap().start, (0, 0));
        assert_eq!(find(&b, (0, 0), "foo", o).unwrap().start, (1, 4));
        let nw = Options { wrap: false, ..o };
        assert!(find(&b, (0, 0), "foo", nw).is_none());
    }

    #[test]
    fn multibyte_columns() {
        let b = Buffer::from_text("héé ñandú ñ");
        let m = find(&b, (0, 0), "ñ", opts()).unwrap();
        assert_eq!(m.start, (0, 4));
        assert_eq!(m.len, 1);
        let m = find(&b, (0, 4), "ñ", opts()).unwrap();
        assert_eq!(m.start, (0, 10));
        assert_eq!(m.end(), (0, 11));
    }

    #[test]
    fn case_toggle() {
        let b = Buffer::from_text("Hello ÉCOLE");
        assert!(find(&b, (0, 0), "hello", opts()).is_none());
        let ci = Options { case_sensitive: false, ..opts() };
        assert_eq!(find(&b, (0, 3), "HELLO", ci).unwrap().start, (0, 0));
        assert_eq!(find(&b, (0, 0), "école", ci).unwrap().start, (0, 6));
    }

    #[test]
    fn no_match_and_empty() {
        let b = Buffer::from_text("abc");
        assert!(find(&b, (0, 0), "zzz", opts()).is_none());
        assert!(find(&b, (0, 0), "", opts()).is_none());
        assert!(find(&b, (0, 0), "a\nb", opts()).is_none());
    }

    #[test]
    fn find_next_moves_cursor() {
        let mut b = Buffer::from_text("ab ab");
        assert!(find_next(&mut b, "ab", opts()).is_some());
        assert_eq!(b.cursor(), (0, 3));
        assert!(find_next(&mut b, "zz", opts()).is_none());
        assert_eq!(b.cursor(), (0, 3));
    }

    #[test]
    fn replace_one_undo_redo() {
        let mut b = Buffer::from_text("aé aé");
        let mut h = History::new();
        assert!(replace_one(&mut b, &mut h, "aé", "XYZ", opts()));
        assert_eq!(b.to_text(), "XYZ aé");
        assert_eq!(b.cursor(), (0, 3));
        assert!(replace_one(&mut b, &mut h, "aé", "", opts()));
        assert_eq!(b.to_text(), "XYZ ");
        assert!(!replace_one(&mut b, &mut h, "aé", "q", opts()));
        h.undo(&mut b);
        assert_eq!(b.to_text(), "XYZ aé");
        h.undo(&mut b);
        assert_eq!(b.to_text(), "aé aé");
        h.redo(&mut b);
        h.redo(&mut b);
        assert_eq!(b.to_text(), "XYZ ");
    }

    #[test]
    fn replace_all_counts_and_undoes() {
        let mut b = Buffer::from_text("ñ a ñ\nÑ ñññ");
        let mut h = History::new();
        let orig = b.to_text();
        let n = replace_all(&mut b, &mut h, "ñ", "nn", false);
        assert_eq!(n, 6);
        assert_eq!(b.to_text(), "nn a nn\nnn nnnnnn");
        assert!(h.undo(&mut b));
        assert_eq!(b.to_text(), orig);
        assert!(!h.can_undo());
        assert!(h.redo(&mut b));
        assert_eq!(b.to_text(), "nn a nn\nnn nnnnnn");
        assert_eq!(replace_all(&mut b, &mut h, "zzz", "y", true), 0);
        assert!(!h.can_redo());
    }

    #[test]
    fn replace_all_case_sensitive() {
        let mut b = Buffer::from_text("Aa aA");
        let mut h = History::new();
        assert_eq!(replace_all(&mut b, &mut h, "a", "b", true), 2);
        assert_eq!(b.to_text(), "Ab bA");
    }
}
