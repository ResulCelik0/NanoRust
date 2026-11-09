//! Hand-written command-line parsing: `nanorust [OPTIONS] [+LINE] FILE`.

/// Parsed run options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub file: String,
    /// 1-based line to start on, from `+LINE`.
    pub line: Option<usize>,
    pub line_numbers: bool,
    pub tab_size: Option<usize>,
    pub tabs_to_spaces: bool,
}

/// Result of parsing the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Run(Options),
    Help,
    Version,
    /// A usage error with a message.
    Error(String),
}

/// Parse arguments (without the program name).
pub fn parse<I, S>(args: I) -> Parsed
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut file: Option<String> = None;
    let mut line = None;
    let mut line_numbers = false;
    let mut tab_size = None;
    let mut tabs_to_spaces = false;
    let mut only_files = false;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let a = a.as_ref();
        if only_files || a == "-" || !(a.starts_with('-') || a.starts_with('+')) {
            if file.is_some() {
                return Parsed::Error(format!("unexpected extra argument '{a}'"));
            }
            file = Some(a.to_string());
            continue;
        }
        if let Some(n) = a.strip_prefix('+') {
            match n.parse::<usize>() {
                Ok(v) if v >= 1 => line = Some(v),
                _ => return Parsed::Error(format!("invalid line number '{n}'")),
            }
            continue;
        }
        match a {
            "--" => only_files = true,
            "--help" | "-h" => return Parsed::Help,
            "--version" | "-V" => return Parsed::Version,
            "--linenumbers" | "-l" => line_numbers = true,
            "--tabstospaces" | "-E" => tabs_to_spaces = true,
            "--tabsize" | "-T" => match it.next() {
                Some(v) => match parse_tab(v.as_ref()) {
                    Ok(n) => tab_size = Some(n),
                    Err(e) => return Parsed::Error(e),
                },
                None => return Parsed::Error(format!("option '{a}' requires an argument")),
            },
            _ => {
                if let Some(v) = a.strip_prefix("--tabsize=") {
                    match parse_tab(v) {
                        Ok(n) => tab_size = Some(n),
                        Err(e) => return Parsed::Error(e),
                    }
                } else if let Some(v) = a.strip_prefix("-T") {
                    match parse_tab(v) {
                        Ok(n) => tab_size = Some(n),
                        Err(e) => return Parsed::Error(e),
                    }
                } else {
                    return Parsed::Error(format!("unknown option '{a}'"));
                }
            }
        }
    }
    match file {
        Some(file) => Parsed::Run(Options {
            file,
            line,
            line_numbers,
            tab_size,
            tabs_to_spaces,
        }),
        None => Parsed::Error("missing file name".to_string()),
    }
}

fn parse_tab(v: &str) -> Result<usize, String> {
    match v.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(format!("invalid tab size '{v}'")),
    }
}

/// Usage/help text.
pub fn help_text() -> String {
    "Usage: nanorust [OPTIONS] [+LINE] FILE\n\
\n\
Options:\n\
\x20 +LINE                 Start at line number LINE\n\
\x20 -l, --linenumbers     Show line numbers\n\
\x20 -T, --tabsize=N       Set tab width to N columns\n\
\x20 -E, --tabstospaces    Convert typed tabs to spaces\n\
\x20 -h, --help            Show this help and exit\n\
\x20 -V, --version         Show version and exit\n\
\x20 --                    Treat the remaining arguments as file names\n"
        .to_string()
}

/// Version text.
pub fn version_text() -> String {
    format!("nanorust {}\n", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Parsed {
        parse(args.iter().copied())
    }

    fn opts(file: &str, line: Option<usize>) -> Parsed {
        Parsed::Run(Options {
            file: file.into(),
            line,
            line_numbers: false,
            tab_size: None,
            tabs_to_spaces: false,
        })
    }

    #[test]
    fn file_only() {
        assert_eq!(run(&["a.txt"]), opts("a.txt", None));
    }

    #[test]
    fn line_and_file() {
        assert_eq!(run(&["+12", "a.txt"]), opts("a.txt", Some(12)));
        assert_eq!(run(&["a.txt", "+3"]), opts("a.txt", Some(3)));
    }

    #[test]
    fn invalid_line() {
        for bad in ["+", "+x", "+0", "+-1", "+1.5"] {
            assert!(matches!(run(&[bad, "f"]), Parsed::Error(_)), "{bad}");
        }
    }

    #[test]
    fn help_and_version() {
        assert_eq!(run(&["--help"]), Parsed::Help);
        assert_eq!(run(&["-h"]), Parsed::Help);
        assert_eq!(run(&["--version"]), Parsed::Version);
        assert_eq!(run(&["-V"]), Parsed::Version);
        assert_eq!(run(&["f", "-h"]), Parsed::Help);
        assert!(help_text().contains("Usage"));
        assert!(version_text().starts_with("nanorust "));
    }

    #[test]
    fn errors() {
        assert!(matches!(run(&[]), Parsed::Error(_)));
        assert!(matches!(run(&["+5"]), Parsed::Error(_)));
        assert!(matches!(run(&["--bogus", "f"]), Parsed::Error(_)));
        assert!(matches!(run(&["-x", "f"]), Parsed::Error(_)));
        assert!(matches!(run(&["a", "b"]), Parsed::Error(_)));
    }

    #[test]
    fn double_dash() {
        assert_eq!(run(&["--", "-weird"]), opts("-weird", None));
        assert_eq!(run(&["+2", "--", "+3"]), opts("+3", Some(2)));
        assert!(matches!(run(&["--"]), Parsed::Error(_)));
        assert!(matches!(run(&["--", "a", "b"]), Parsed::Error(_)));
        assert_eq!(run(&["--", "--help"]), opts("--help", None));
    }

    #[test]
    fn dash_is_a_file() {
        assert_eq!(run(&["-"]), opts("-", None));
    }

    #[test]
    fn flags() {
        let p = run(&["-l", "-E", "-T", "4", "f"]);
        assert_eq!(
            p,
            Parsed::Run(Options {
                file: "f".into(),
                line: None,
                line_numbers: true,
                tab_size: Some(4),
                tabs_to_spaces: true
            })
        );
        assert!(matches!(run(&["--tabsize=8", "f"]), Parsed::Run(o) if o.tab_size == Some(8)));
        assert!(matches!(run(&["-T2", "f"]), Parsed::Run(o) if o.tab_size == Some(2)));
        assert!(matches!(run(&["-T"]), Parsed::Error(_)));
        assert!(matches!(run(&["-T", "0", "f"]), Parsed::Error(_)));
        assert!(matches!(run(&["--tabsize=z", "f"]), Parsed::Error(_)));
    }
}
