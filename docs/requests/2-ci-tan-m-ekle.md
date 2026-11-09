# Request #2: add a CI definition (CI tanımı ekle)

User request (Turkish): add a CI flow that runs `cargo test`.

## Interpretation
A GitHub Actions workflow that runs `cargo test` on every push and pull request. Workploy's own
pre-PR check, `.workploy/ci.yml`, already runs `cargo build --all-targets` and `cargo test`, so it
needs no change.

## Constraint
Agents may not create or change anything under `.github/`. The workflow is therefore delivered as
`docs/ci/github-actions.yml`, and the README tells the maintainer to copy it to
`.github/workflows/ci.yml` (one `mkdir` + `cp`). Until copied, GitHub does not run it.

## Design decisions
- Single job on `ubuntu-latest`, stable Rust via the preinstalled toolchain (no third-party
  toolchain action; only `actions/checkout`).
- Steps: `cargo build --all-targets`, `cargo test` (same as `.workploy/ci.yml`, so the two stay equivalent).
- Cache `~/.cargo/registry` and `target` with `actions/cache`, keyed on `Cargo.lock`.
- The e2e tests use `script` for a pty; it ships with `util-linux` on ubuntu runners.
- fmt/clippy are not added: they are not installable here, so they cannot be verified. Can be a later request.
- No code or dependency changes.

## Stage 6: CI workflow definition
Goal: a ready-to-install GitHub Actions workflow running `cargo test`, documented.
- [ ] Add `docs/ci/github-actions.yml` (triggers: push, pull_request; checkout, cache, build, test)
- [ ] README "Continuous integration" section: copy instructions, what it runs
- [ ] Confirm `.workploy/ci.yml` still runs build + test (unchanged)
- [ ] Update CLAUDE.md with a one-line note on the CI file location

Acceptance: the YAML parses (checked with a YAML parser, e.g. `python3 -c "import yaml"` if
available, else careful review); its `run` commands are executed locally and pass
(`cargo build --all-targets`, `cargo test`); nothing under `.github/` is touched.
