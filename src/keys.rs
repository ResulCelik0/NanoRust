//! Mapping of crossterm key events to editor [`Action`]s using nano's bindings.
//!
//! This is the only core module that touches crossterm, and only its event types.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

/// A high-level editor command, independent of the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Insert a printable character (or type into a prompt).
    Insert(char),
    /// Enter: line break in the buffer, confirm in a prompt.
    Newline,
    /// Tab: inserts a tab character.
    Tab,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    /// Ctrl+A / Home.
    Home,
    /// Ctrl+E / End.
    End,
    /// Ctrl+Y / PgUp.
    PageUp,
    /// Ctrl+V / PgDn.
    PageDown,
    /// Ctrl+K: cut the current line (or selection).
    Cut,
    /// Ctrl+U: paste the cut buffer.
    Paste,
    /// Ctrl+O: write out (save).
    WriteOut,
    /// Ctrl+X: exit.
    Exit,
    /// Ctrl+C: cancel the current prompt.
    Cancel,
    /// Alt+U.
    Undo,
    /// Alt+E.
    Redo,
    /// Terminal resized to `(columns, rows)`.
    Resize(u16, u16),
}

/// Maps a key event to an [`Action`]; `None` for unbound keys and key releases.
pub fn map_key(key: KeyEvent) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Char(c) if ctrl => match c.to_ascii_lowercase() {
            'o' => Some(Action::WriteOut),
            'x' => Some(Action::Exit),
            'k' => Some(Action::Cut),
            'u' => Some(Action::Paste),
            'a' => Some(Action::Home),
            'e' => Some(Action::End),
            'y' => Some(Action::PageUp),
            'v' => Some(Action::PageDown),
            'c' => Some(Action::Cancel),
            _ => None,
        },
        KeyCode::Char(c) if alt => match c.to_ascii_lowercase() {
            'u' => Some(Action::Undo),
            'e' => Some(Action::Redo),
            _ => None,
        },
        KeyCode::Char(c) => Some(Action::Insert(c)),
        KeyCode::Enter => Some(Action::Newline),
        KeyCode::Tab => Some(Action::Tab),
        KeyCode::Backspace => Some(Action::Backspace),
        KeyCode::Delete => Some(Action::Delete),
        KeyCode::Left => Some(Action::Left),
        KeyCode::Right => Some(Action::Right),
        KeyCode::Up => Some(Action::Up),
        KeyCode::Down => Some(Action::Down),
        KeyCode::Home => Some(Action::Home),
        KeyCode::End => Some(Action::End),
        KeyCode::PageUp => Some(Action::PageUp),
        KeyCode::PageDown => Some(Action::PageDown),
        _ => None,
    }
}

/// Maps any terminal event to an [`Action`] (key presses and resizes).
pub fn map_event(event: &Event) -> Option<Action> {
    match event {
        Event::Key(k) => map_key(*k),
        Event::Resize(w, h) => Some(Action::Resize(*w, *h)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode, m: KeyModifiers) -> Option<Action> {
        map_key(KeyEvent::new(code, m))
    }
    const N: KeyModifiers = KeyModifiers::NONE;
    const C: KeyModifiers = KeyModifiers::CONTROL;

    #[test]
    fn ctrl_bindings() {
        let c = |ch| k(KeyCode::Char(ch), C);
        assert_eq!(c('o'), Some(Action::WriteOut));
        assert_eq!(c('x'), Some(Action::Exit));
        assert_eq!(c('k'), Some(Action::Cut));
        assert_eq!(c('u'), Some(Action::Paste));
        assert_eq!(c('a'), Some(Action::Home));
        assert_eq!(c('e'), Some(Action::End));
        assert_eq!(c('y'), Some(Action::PageUp));
        assert_eq!(c('v'), Some(Action::PageDown));
        assert_eq!(c('c'), Some(Action::Cancel));
        assert_eq!(c('q'), None);
    }

    #[test]
    fn alt_bindings() {
        let a = |ch| k(KeyCode::Char(ch), KeyModifiers::ALT);
        assert_eq!(a('u'), Some(Action::Undo));
        assert_eq!(a('e'), Some(Action::Redo));
        assert_eq!(a('z'), None);
    }

    #[test]
    fn plain_keys() {
        assert_eq!(k(KeyCode::Char('a'), N), Some(Action::Insert('a')));
        assert_eq!(
            k(KeyCode::Char('A'), KeyModifiers::SHIFT),
            Some(Action::Insert('A'))
        );
        assert_eq!(k(KeyCode::Char('é'), N), Some(Action::Insert('é')));
        assert_eq!(k(KeyCode::Enter, N), Some(Action::Newline));
        assert_eq!(k(KeyCode::Tab, N), Some(Action::Tab));
        assert_eq!(k(KeyCode::Backspace, N), Some(Action::Backspace));
        assert_eq!(k(KeyCode::Delete, N), Some(Action::Delete));
        assert_eq!(k(KeyCode::Left, N), Some(Action::Left));
        assert_eq!(k(KeyCode::Right, N), Some(Action::Right));
        assert_eq!(k(KeyCode::Up, N), Some(Action::Up));
        assert_eq!(k(KeyCode::Down, N), Some(Action::Down));
        assert_eq!(k(KeyCode::Home, N), Some(Action::Home));
        assert_eq!(k(KeyCode::End, N), Some(Action::End));
        assert_eq!(k(KeyCode::PageUp, N), Some(Action::PageUp));
        assert_eq!(k(KeyCode::PageDown, N), Some(Action::PageDown));
        assert_eq!(k(KeyCode::F(1), N), None);
    }

    #[test]
    fn releases_ignored_and_resize() {
        let mut ev = KeyEvent::new(KeyCode::Char('a'), N);
        ev.kind = KeyEventKind::Release;
        assert_eq!(map_key(ev), None);
        assert_eq!(
            map_event(&Event::Resize(80, 24)),
            Some(Action::Resize(80, 24))
        );
        assert_eq!(map_event(&Event::FocusGained), None);
    }
}
