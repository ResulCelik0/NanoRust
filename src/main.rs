use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;

use crossterm::event;
use nanorust::args::{self, Parsed};
use nanorust::editor::Editor;
use nanorust::keys::map_event;
use nanorust::term::{install_panic_hook, Guard};
use nanorust::ui::{self, Screen};

fn run(mut ed: Editor) -> io::Result<()> {
    let _guard = Guard::new()?;
    let mut out = io::stdout();
    let mut prev: Option<Screen> = None;
    loop {
        let (w, h) = crossterm::terminal::size()?;
        let (w, h) = (w as usize, h as usize);
        ui::prepare(&mut ed, w, h);
        let screen = ui::render(&ed, w, h);
        ui::draw(&mut out, &screen, prev.as_ref())?;
        prev = Some(screen);
        if ed.should_quit() {
            return Ok(());
        }
        let ev = event::read()?;
        if let Some(action) = map_event(&ev) {
            ed.handle(action);
        }
    }
}

fn main() -> ExitCode {
    let opts = match args::parse(std::env::args().skip(1)) {
        Parsed::Help => {
            print!("{}", args::help_text());
            return ExitCode::SUCCESS;
        }
        Parsed::Version => {
            print!("{}", args::version_text());
            return ExitCode::SUCCESS;
        }
        Parsed::Error(msg) => {
            eprintln!("nanorust: {msg}");
            eprintln!("Try 'nanorust --help' for more information.");
            return ExitCode::from(2);
        }
        Parsed::Run(o) => o,
    };
    let mut ed = match Editor::open(Path::new(&opts.file), opts.line) {
        Ok(ed) => ed,
        Err(e) => {
            eprintln!("nanorust: {}: {e}", opts.file);
            return ExitCode::from(1);
        }
    };
    ed.configure(opts.tab_size, opts.tabs_to_spaces, opts.line_numbers);
    install_panic_hook();
    match run(ed) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            let _ = io::stdout().flush();
            eprintln!("nanorust: {e}");
            ExitCode::from(1)
        }
    }
}
