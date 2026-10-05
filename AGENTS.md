# AGENTS.md

Guidance for humans and AI agents working on Koda, a **terminal-native IDE**.
It is not a terminal copy of VS Code. Every feature must justify its existence
against the principles below.

The user installs Koda, opens a project, and codes — without configuring language
servers, plugins or keybindings.

---

## 1. Core principles (non-negotiable)

1. **Zero configuration.** Supported languages work with no user setup.
   Complexity belongs inside Koda, never in a config file the user writes.
2. **Koda owns language intelligence.** Detection and language providers are
   first-class. Do not scatter `if rust { … }` through the editor or UI.
3. **Terminal-native.** Never paint an opaque full-screen background. Use
   `Color::Reset` for plain text and set backgrounds only where meaningful
   (selections, overlays).
4. **Keyboard-first and familiar.** Prefer conventional shortcuts. `Ctrl+S`
   must always save.
5. **Performance is a feature.** Never block the UI on expensive work.

If a design decision conflicts with these principles, the principles win.

---

## 2. Architecture

The single source tree, dependency rules, how to add a language, and the
detection-signal model live in **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**.

Short version:

- `editor`, `filesystem`, `git`, `commands` must not depend on `ui` or `app`.
- `language` depends on nothing UI-related; the editor core stays
  language-agnostic.
- `ui` reads `app`/editor/language data but contains no editing logic.
- `app` is the conductor and may use everything.

Adding a language means implementing `LanguageProvider` and registering it in
`ProviderRegistry::builtin()` — nothing in `editor` or `ui` changes.

---

## 3. Where the rationale lives

`AGENTS.md` is rules only. The *why* is in:

- **[docs/DECISIONS.md](docs/DECISIONS.md)** — read the relevant section before
  changing a subsystem:
  - [Editor](docs/DECISIONS.md#editor) before `editor/`
  - [Language and detection](docs/DECISIONS.md#language-and-detection) before
    `language/<name>/`, `language/format.rs`, `language/detection/`
  - [LSP](docs/DECISIONS.md#lsp) before `language/lsp/`
  - [Provisioning](docs/DECISIONS.md#provisioning) before `language/tools.rs`
  - [UI](docs/DECISIONS.md#ui) before `ui/`
  - [Project, search and git](docs/DECISIONS.md#project-search-and-git) before
    `project/`, `search.rs`, `git/`
- **[docs/LIMITATIONS.md](docs/LIMITATIONS.md)** — what does not work yet.

---

## 4. Coding conventions

- Rust edition **2024**, stable toolchain.
- Run `cargo fmt` and `cargo clippy --all-targets` before finishing. Clippy must
  be warning-free.
- Prefer clear, small modules over giant files. **No source file may exceed 800
  lines.** New code must not push a file past the limit; if a file is already
  over, see the `src/app/mod.rs` split in
  [DECISIONS.md](docs/DECISIONS.md#splitting-srcappmodrs).
- Use `Result`/`Option` and propagate errors; do not `unwrap()` on user input or
  filesystem operations (tests and proven invariants are fine).
- Comments explain *why*, not *what*. Stale comments are worse than none.
- Public items get doc comments explaining intent.
- UI strings use plain ASCII except intentional glyphs (`▸ ▾ │ ● ⎇ ❯ ✦ ☾`).
- Measure positions in **characters**, not bytes.
- Run external processes through `process::wait_captured`; never
  `Command::output`/`wait_with_output` in a new call site.
- Convert LSP positions only in `language/lsp/convert.rs`; never do ad-hoc
  character/byte arithmetic in the UI.
- Do not add a dependency without listing it and justifying it first.

---

## 5. Platform

**Linux and macOS are supported; Windows is not yet supported.** There is no
Windows CI and several provisioning plans require a Unix `sh`. Do not add
Windows-only code paths or claim Windows support. See
[LIMITATIONS.md](docs/LIMITATIONS.md#platform).

---

## 6. Never do this

- Never write to the user's system directories or modify their shell startup
  files.
- Never make a managed download without an integrity check: every fixed-artifact
  download is verified, and a new one must supply a digest. Package-manager
  installs are pinned to an exact version and delegate integrity to the manager
  (see LIMITATIONS.md).
- Never paint an opaque full-screen background.
- Never let language-specific logic leak into `editor/` or `ui/`.
- Never require a config file for a core feature.
- Never use `unwrap()` on user input or filesystem results.
- Never break an existing test to make progress.
- Never begin the `src/app/mod.rs` split without an approved proposal.

---

## 7. Development workflow

Work commit-by-commit. Each meaningful commit must:

1. Have a single clear purpose.
2. Compile (`cargo check`).
3. Pass tests (`cargo test`).
4. Be free of clippy warnings.
5. Leave the repository usable.

Before changing code: read the modules you will touch, follow surrounding
conventions, and keep changes focused. Update `CHANGELOG.md` for user-visible
changes and `ROADMAP.md` when direction changes. Keep `README.md` and this file
in sync.

Tests for new behaviour:

- Editor/buffer logic: unit tests next to the code.
- Detection/providers: unit tests in their modules.
- UI composition: `tests/render.rs` with ratatui's `TestBackend`.
- Live language servers: `tests/optional_servers.rs` (and the tooling suites),
  every test `#[ignore]`d and skipping when its tool is absent.

---

## 8. Commands

```bash
cargo check                     # fast compile check
cargo test                      # all tests
cargo clippy --all-targets      # lints (must be clean)
cargo fmt                       # formatting
cargo run -- .                  # run against this project
cargo run -- src/main.rs        # run against a file
```

Managed tools can be installed for the live tests with:

```bash
cargo run --example install_check -- <tool>   # e.g. gopls, kotlin, lua, dart
cargo test --test optional_servers -- --ignored --test-threads=1 --nocapture
```
