# NanoRust

Rust clone of the nano editor. See docs/design.md (architecture) and docs/plan.md (stages).

## Commands
- Build: `cargo build` (release: `cargo build --release`)
- Test: `cargo test`
- Lint: `cargo fmt --check && cargo clippy --all-targets -- -D warnings`
- Run: `cargo run -- [+LINE] FILE`

## Conventions
- Single crate: logic in the library (`src/lib.rs` + modules), `src/main.rs` stays thin.
- Core modules must not touch the terminal; only `ui`/`main` use crossterm.
- Dependencies: crossterm, unicode-width only; ask before adding more.
- No `unwrap()` on I/O or user input in non-test code; handle UTF-8 by char boundaries.
- Every feature ships with tests; keep fmt and clippy clean.
- Note: Rust toolchain may need installing (rustup via crates.io-reachable proxy or distro package) — check `cargo --version` first.
- Commit messages: short imperative; keep docs in sync with plan checkboxes.
