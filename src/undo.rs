//! Undo/redo history.
//!
//! Operation based: the history stores groups of buffer [`Edit`]s. One undo step
//! reverts a whole group (e.g. a replace is a delete plus an insert). Recording
//! a new edit clears the redo stack.

use crate::buffer::{Buffer, Edit};

/// Undo/redo stacks of edit groups.
#[derive(Debug, Default, Clone)]
pub struct History {
    undo: Vec<Vec<Edit>>,
    redo: Vec<Vec<Edit>>,
}

impl History {
    /// Creates an empty history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an edit that has already been applied to the buffer.
    pub fn record(&mut self, edit: Edit) {
        self.record_group(vec![edit]);
    }

    /// Records the `Option<Edit>` returned by a buffer operation (no-op for `None`).
    pub fn record_opt(&mut self, edit: Option<Edit>) {
        if let Some(e) = edit {
            self.record(e);
        }
    }

    /// Records several already-applied edits (in application order) as one step.
    pub fn record_group(&mut self, edits: Vec<Edit>) {
        if edits.is_empty() {
            return;
        }
        self.redo.clear();
        self.undo.push(edits);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Reverts the most recent step. Returns `false` if there is nothing to undo.
    pub fn undo(&mut self, buf: &mut Buffer) -> bool {
        let Some(group) = self.undo.pop() else {
            return false;
        };
        for e in group.iter().rev() {
            buf.apply(&e.inverse());
        }
        self.redo.push(group);
        true
    }

    /// Re-applies the most recently undone step. Returns `false` if none.
    pub fn redo(&mut self, buf: &mut Buffer) -> bool {
        let Some(group) = self.redo.pop() else {
            return false;
        };
        for e in &group {
            buf.apply(e);
        }
        self.undo.push(group);
        true
    }

    /// Forgets all history (e.g. after loading a new file).
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_delete_round_trip() {
        let mut b = Buffer::from_text("héllo\nworld");
        let mut h = History::new();
        let orig = b.to_text();
        b.set_cursor((0, 2));
        h.record_opt(b.insert_str("ñ\nü"));
        h.record_opt(b.backspace());
        h.record_opt(b.delete());
        let done = b.to_text();
        assert_ne!(done, orig);
        while h.undo(&mut b) {}
        assert_eq!(b.to_text(), orig);
        while h.redo(&mut b) {}
        assert_eq!(b.to_text(), done);
    }

    #[test]
    fn cut_and_paste_round_trip() {
        let mut b = Buffer::from_text("one\ntwo\nthree");
        let mut h = History::new();
        let orig = b.to_text();
        b.set_cursor((1, 0));
        h.record_opt(b.cut_line());
        h.record_opt(b.paste());
        b.set_cursor((0, 1));
        h.record_opt(b.paste());
        let done = b.to_text();
        assert!(h.undo(&mut b));
        assert!(h.undo(&mut b));
        assert!(h.undo(&mut b));
        assert_eq!(b.to_text(), orig);
        assert!(!h.undo(&mut b));
        while h.redo(&mut b) {}
        assert_eq!(b.to_text(), done);
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut b = Buffer::from_text("");
        let mut h = History::new();
        h.record_opt(b.insert_char('a'));
        assert!(h.undo(&mut b));
        assert!(h.can_redo());
        h.record_opt(b.insert_char('b'));
        assert!(!h.can_redo());
        assert!(!h.redo(&mut b));
        assert_eq!(b.to_text(), "b");
    }

    #[test]
    fn group_is_one_step() {
        let mut b = Buffer::from_text("abc");
        let mut h = History::new();
        let e1 = Edit::Delete {
            pos: (0, 0),
            text: "a".into(),
        };
        let e2 = Edit::Insert {
            pos: (0, 0),
            text: "xy".into(),
        };
        b.apply(&e1);
        b.apply(&e2);
        h.record_group(vec![e1, e2]);
        assert_eq!(b.to_text(), "xybc");
        assert!(h.undo(&mut b));
        assert_eq!(b.to_text(), "abc");
        assert!(!h.can_undo());
        assert!(h.redo(&mut b));
        assert_eq!(b.to_text(), "xybc");
    }
}
