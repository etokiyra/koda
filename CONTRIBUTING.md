# Contributing to Koda

Thanks for helping. Koda is a zero-configuration terminal IDE; the bar for a
change is that it keeps the editor core clean, the UI minimal, and the
experience configuration-free.

## Before you start

- Read [`AGENTS.md`](AGENTS.md) — the principles, rules and conventions.
- Read the relevant section of [`docs/DECISIONS.md`](docs/DECISIONS.md) before
  changing a subsystem, and [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for
  the module layout and dependency rules.
- Check [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md) so you do not re-litigate a
  known limitation by accident.

## Set up

```bash
git clone https://github.com/etokiyra/koda.git
cd koda
cargo build
cargo run -- .          # run Koda against this repository
```

## The gates

Every commit must pass all of these:

```bash
cargo fmt --check
cargo clippy --all-targets          # must be warning-free
cargo test                          # unit + render tests
```

The live language-server suite is `#[ignore]`d and skips when a tool is absent.
To run it, install a managed tool first and then run the ignored tests:

```bash
cargo run --example install_check -- gopls   # or kotlin, lua, dart, …
cargo test --test optional_servers -- --ignored --test-threads=1 --nocapture
```

## Workflow

- **One purpose per commit.** Keep commits small and focused.
- Add tests for new behaviour: unit tests next to editor/detection/provider
  code, and `tests/render.rs` for UI composition.
- Update [`CHANGELOG.md`](CHANGELOG.md) under the appropriate Keep a Changelog
  group for any user-visible change, and [`ROADMAP.md`](ROADMAP.md) if you
  complete or change direction.
- Keep `README.md`, `AGENTS.md` and the docs in sync.

## Rules of the codebase

- **No source file may exceed 800 lines.** New code must not push a file past
  the limit; if a file is already over, prefer extracting a module.
- Language-specific logic never leaks into `editor/` or `ui/`. Add a language by
  implementing the `LanguageProvider` trait and registering it — see
  [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md#adding-a-language).
- Run external processes through `process::wait_captured`, and convert LSP
  positions only in `language/lsp/convert.rs`.
- Do not add a dependency without listing it and justifying it in an issue or
  pull request first.
- Never require a config file for a core feature, never paint an opaque
  full-screen background, and never write to a system-owned location.

## Keymap

The README keymap is generated from the command registry. After adding or
changing a command shortcut:

```bash
cargo run --example keymap           # regenerate the table
# paste it between the keymap:start/keymap:end markers in README.md
cargo test --test keymap             # fails if the README drifts
```

## Reporting issues

Include your OS and architecture, the Koda version, the exact steps, and what you
expected. For anything security-sensitive, follow [`SECURITY.md`](SECURITY.md)
instead of opening a public issue.
