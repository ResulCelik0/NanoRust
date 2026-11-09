//! File reading and writing.
//!
//! Independent of buffer internals: documents are plain `Vec<String>` lines
//! without line terminators. A loaded document always has at least one line
//! (an empty file loads as `[""]`, no final newline).
//! Invalid UTF-8 is reported as an `InvalidData` error (never lossy).

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub lines: Vec<String>,
    pub line_ending: LineEnding,
    pub final_newline: bool,
    /// True when the file did not exist.
    pub is_new: bool,
}

/// Load a file. A missing file yields an empty new document.
/// Line ending is CRLF if the first line break is CRLF, else LF.
pub fn load(path: &Path) -> io::Result<Loaded> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Loaded {
                lines: vec![String::new()],
                line_ending: LineEnding::Lf,
                final_newline: false,
                is_new: true,
            });
        }
        Err(e) => return Err(e),
    };
    let text = String::from_utf8(bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("invalid UTF-8: {e}")))?;
    Ok(parse(&text))
}

fn parse(text: &str) -> Loaded {
    let line_ending = match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes()[i - 1] == b'\r' => LineEnding::CrLf,
        _ => LineEnding::Lf,
    };
    let final_newline = text.ends_with('\n');
    let body = if final_newline {
        &text[..text.len() - 1]
    } else {
        text
    };
    let mut lines: Vec<String> = body
        .split('\n')
        .map(|l| {
            if line_ending == LineEnding::CrLf {
                l.strip_suffix('\r').unwrap_or(l).to_string()
            } else {
                l.to_string()
            }
        })
        .collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    Loaded {
        lines,
        line_ending,
        final_newline,
        is_new: false,
    }
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = format!(".{name}.nanorust-{}.tmp", std::process::id());
    path.with_file_name(tmp)
}

/// Atomically save: write a temp file in the same directory, then rename.
/// Permissions of an existing target are preserved; the temp file is removed on failure.
pub fn save(
    path: &Path,
    lines: &[String],
    line_ending: LineEnding,
    final_newline: bool,
) -> io::Result<()> {
    let tmp = temp_path(path);
    let result = write_and_rename(&tmp, path, lines, line_ending, final_newline);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_and_rename(
    tmp: &Path,
    path: &Path,
    lines: &[String],
    line_ending: LineEnding,
    final_newline: bool,
) -> io::Result<()> {
    let eol = line_ending.as_str();
    let mut data = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            data.push_str(eol);
        }
        data.push_str(l);
    }
    if final_newline {
        data.push_str(eol);
    }
    let perms = fs::metadata(path).ok().map(|m| m.permissions());
    let mut f = fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
    f.write_all(data.as_bytes())?;
    if let Some(p) = perms {
        f.set_permissions(p)?;
    }
    f.sync_all()?;
    drop(f);
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new(tag: &str) -> Dir {
            let p = std::env::temp_dir().join(format!(
                "nanorust-fileio-{tag}-{}-{:?}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            fs::create_dir_all(&p).unwrap();
            Dir(p)
        }
        fn file(&self, n: &str) -> PathBuf {
            self.0.join(n)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn lf_round_trip() {
        let d = Dir::new("lf");
        let p = d.file("a.txt");
        fs::write(&p, "a\nb\n").unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.lines, v(&["a", "b"]));
        assert_eq!(l.line_ending, LineEnding::Lf);
        assert!(l.final_newline && !l.is_new);
        save(&p, &l.lines, l.line_ending, l.final_newline).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"a\nb\n");
    }

    #[test]
    fn crlf_round_trip() {
        let d = Dir::new("crlf");
        let p = d.file("a.txt");
        fs::write(&p, "a\r\n\r\nb\r\n").unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.lines, v(&["a", "", "b"]));
        assert_eq!(l.line_ending, LineEnding::CrLf);
        save(&p, &l.lines, l.line_ending, l.final_newline).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"a\r\n\r\nb\r\n");
    }

    #[test]
    fn no_final_newline() {
        let d = Dir::new("nofn");
        let p = d.file("a.txt");
        fs::write(&p, "a\r\nb").unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.lines, v(&["a", "b"]));
        assert!(!l.final_newline);
        save(&p, &l.lines, l.line_ending, l.final_newline).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"a\r\nb");
    }

    #[test]
    fn empty_file() {
        let d = Dir::new("empty");
        let p = d.file("a.txt");
        fs::write(&p, "").unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.lines, v(&[""]));
        assert!(!l.final_newline && !l.is_new);
        save(&p, &l.lines, l.line_ending, l.final_newline).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"");
        fs::write(&p, "\n").unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.lines, v(&[""]));
        assert!(l.final_newline);
    }

    #[test]
    fn multibyte() {
        let d = Dir::new("mb");
        let p = d.file("a.txt");
        let lines = v(&["héllo", "日本語", "😀 x"]);
        save(&p, &lines, LineEnding::Lf, true).unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.lines, lines);
    }

    #[test]
    fn missing_file_is_new() {
        let d = Dir::new("missing");
        let l = load(&d.file("nope")).unwrap();
        assert!(l.is_new);
        assert_eq!(l.lines, v(&[""]));
        assert!(!l.final_newline);
    }

    #[test]
    fn invalid_utf8_errors() {
        let d = Dir::new("bad");
        let p = d.file("a.bin");
        fs::write(&p, [0x61, 0xff, 0xfe, 0x0a]).unwrap();
        let e = load(&p).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn save_load_identity_and_overwrite() {
        let d = Dir::new("ident");
        let p = d.file("a.txt");
        save(&p, &v(&["long old content", "more"]), LineEnding::Lf, true).unwrap();
        let lines = v(&["x", "", "y"]);
        save(&p, &lines, LineEnding::CrLf, false).unwrap();
        let l = load(&p).unwrap();
        assert_eq!(l.lines, lines);
        assert_eq!(l.line_ending, LineEnding::CrLf);
        assert!(!l.final_newline);
        // no temp files left behind
        assert_eq!(fs::read_dir(&d.0).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let d = Dir::new("perm");
        let p = d.file("a.sh");
        fs::write(&p, "old").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o751)).unwrap();
        save(&p, &v(&["new"]), LineEnding::Lf, true).unwrap();
        let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o751);
    }

    #[test]
    fn failure_cleans_temp() {
        let d = Dir::new("fail");
        // target is a non-empty directory: rename fails after temp is written
        let p = d.file("target");
        fs::create_dir(&p).unwrap();
        fs::write(p.join("x"), "x").unwrap();
        assert!(save(&p, &v(&["a"]), LineEnding::Lf, true).is_err());
        assert_eq!(fs::read_dir(&d.0).unwrap().count(), 1);
    }
}
