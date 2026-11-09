//! End-to-end tests driving the real binary.
//!
//! The editor needs a terminal, so interactive scenarios run it under a pty
//! via the util-linux `script` command (no scripted-events mode exists in
//! main). Keystrokes are written to `script`'s stdin with short pauses.
//! If `script` is unavailable, interactive tests are skipped.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_nanorust");

fn have_script() -> bool {
    Command::new("script")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nanorust-e2e-{}-{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs the editor in a pty, sending each key chunk with a pause; returns exit success.
fn drive(args: &[&str], keys: &[&[u8]]) -> bool {
    let cmd = format!("exec '{}' {}", BIN, args.join(" "));
    let mut child = Command::new("script")
        .args(["-q", "-e", "-c", &cmd, "/dev/null"])
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    sleep(Duration::from_millis(600));
    for k in keys {
        let _ = stdin.write_all(k);
        let _ = stdin.flush();
        sleep(Duration::from_millis(250));
    }
    let start = Instant::now();
    loop {
        if let Some(st) = child.try_wait().unwrap() {
            return st.success();
        }
        if start.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            panic!("editor did not exit");
        }
        sleep(Duration::from_millis(50));
    }
}

#[test]
fn version_and_help() {
    for flag in ["--version", "--help"] {
        let o = Command::new(BIN).arg(flag).output().unwrap();
        assert!(o.status.success(), "{flag}");
        let out = String::from_utf8_lossy(&o.stdout);
        assert!(out.contains("nanorust"), "{flag}: {out}");
        if flag == "--version" {
            assert!(out.contains(env!("CARGO_PKG_VERSION")));
        }
    }
}

#[test]
fn bad_option_fails() {
    let o = Command::new(BIN).arg("--bogus").output().unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn type_save_and_exit() {
    if !have_script() {
        return;
    }
    let f = tmpdir("save").join("a.txt");
    std::fs::write(&f, "world\n").unwrap();
    let ok = drive(
        &[f.to_str().unwrap()],
        &[b"hello ", b"\x0f", b"\r", b"\x18"],
    );
    assert!(ok);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "hello world\n");
}

#[test]
fn exit_unsaved_answer_no_leaves_file() {
    if !have_script() {
        return;
    }
    let f = tmpdir("nosave").join("a.txt");
    std::fs::write(&f, "orig\n").unwrap();
    let ok = drive(&[f.to_str().unwrap()], &[b"zzz", b"\x18", b"n"]);
    assert!(ok);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "orig\n");
}

#[test]
fn plus_line_positions_cursor() {
    if !have_script() {
        return;
    }
    let f = tmpdir("line").join("a.txt");
    std::fs::write(&f, "one\ntwo\nthree\n").unwrap();
    let ok = drive(
        &["+2", f.to_str().unwrap()],
        &[b"X", b"\x0f", b"\r", b"\x18"],
    );
    assert!(ok);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "one\nXtwo\nthree\n");
}

#[test]
fn search_replace_goto_cut_paste_undo_save() {
    if !have_script() {
        return;
    }
    let f = tmpdir("scenario").join("a.txt");
    std::fs::write(&f, "alpha beta\nbeta gamma\nline three\nlast\n").unwrap();
    let ok = drive(
        &[f.to_str().unwrap()],
        &[
            b"\x17", // Ctrl+W search
            b"gamma\r", // find it (cursor leaves line 1)
            b"\x1f1\r", // goto line 1
            b"\x1c", // replace
            b"beta\r",
            b"BETA\r",
            b"a", // replace all
            b"\x1f3\r", // goto line 3
            b"\x0b", // Ctrl+K cut "line three"
            b"\x1f1\r", // goto line 1
            b"\x15", // Ctrl+U paste above line 1
            b"\x1bu", // Alt+U undo the paste
            b"\x0f",
            b"\r",
            b"\x18",
        ],
    );
    assert!(ok);
    assert_eq!(
        std::fs::read_to_string(&f).unwrap(),
        "alpha BETA\nBETA gamma\nlast\n"
    );
}

#[test]
fn tabs_to_spaces_option() {
    if !have_script() {
        return;
    }
    let f = tmpdir("etabs").join("a.txt");
    std::fs::write(&f, "x\n").unwrap();
    let ok = drive(
        &["-E", "-T", "4", f.to_str().unwrap()],
        &[b"\t", b"\x0f", b"\r", b"\x18"],
    );
    assert!(ok);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "    x\n");
}

#[test]
fn line_numbers_option_edits_normally() {
    if !have_script() {
        return;
    }
    let f = tmpdir("lnum").join("a.txt");
    std::fs::write(&f, "one\ntwo\n").unwrap();
    let ok = drive(
        &["-l", "+2", f.to_str().unwrap()],
        &[b"X", b"\x0f", b"\r", b"\x18"],
    );
    assert!(ok);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "one\nXtwo\n");
}
