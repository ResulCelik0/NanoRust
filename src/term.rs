//! Terminal setup/teardown (raw mode + alternate screen) via crossterm.

use std::io::{self, Write};

use crossterm::{
    cursor::Show,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

/// Enters raw mode and the alternate screen; restores both on drop.
pub struct Guard {
    _private: (),
}

impl Guard {
    pub fn new() -> io::Result<Guard> {
        enable_raw_mode()?;
        if let Err(e) = execute!(io::stdout(), EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(e);
        }
        Ok(Guard { _private: () })
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        restore();
    }
}

/// Best-effort restore of the terminal; safe to call more than once.
pub fn restore() {
    let mut out = io::stdout();
    let _ = execute!(out, LeaveAlternateScreen, Show);
    let _ = disable_raw_mode();
    let _ = out.flush();
}

/// Install a panic hook that restores the terminal before the message is printed.
pub fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_is_idempotent_without_tty() {
        restore();
        restore();
    }

    #[test]
    fn panic_hook_installs_and_restores() {
        install_panic_hook();
        let r = std::panic::catch_unwind(|| panic!("boom"));
        assert!(r.is_err());
    }
}
