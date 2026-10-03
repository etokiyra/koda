# AGENTS.md

Guidance for humans and AI agents working on Koda.

Koda is a **terminal-native IDE**. It is not a terminal copy of VS Code. Every
feature must justify its existence. The user installs Koda, opens a project, and
codes — without configuring language servers, plugins or keybindings.

---

## 1. Core principles (non-negotiable)

1. **Zero configuration.** Supported languages must work with no user setup.
   Complexity belongs inside Koda, never in a config file the user has to write.
2. **Koda owns language intelligence.** Detection and language providers are
   first-class. Do not scatter language checks (`if rust { … }`) through the
   editor or UI.
3. **Terminal-native.** Never fake transparency. Do not paint an opaque
   background over the whole UI. Use `Color::Reset` for plain text and only set
   backgrounds where they are meaningful (selections, overlays).
4. **Keyboard-first and familiar.** Prefer conventional shortcuts.
   `Ctrl+S` must always save.
5. **Performance is a feature.** Never block the UI on expensive work.

If a design decision conflicts with these principles, the principles win.

---

## 2. Architecture

```
src/
├── main.rs               # CLI entry point (thin)
├── lib.rs                # module tree
├── app/
│   ├── mod.rs            # App state, event loop, command dispatch
│   └── overlay.rs        # picker/prompt/search state + fuzzy matching
├── commands/mod.rs       # command registry (ids, titles, shortcuts)
├── editor/
│   ├── buffer.rs         # rope-backed text buffer
│   ├── document.rs       # buffer + cursor + selection + undo + highlight cache
│   ├── history.rs        # undo/redo edits
│   ├── position.rs       # Position / Selection
│   └── mod.rs            # Editor (open documents / tabs)
├── filesystem/mod.rs     # fs helpers (sorted reads, file walking)
├── git/mod.rs            # branch + file status via the `git` binary
├── language/
│   ├── id.rs             # LanguageId
│   ├── detection/        # engine, signals, confidence
│   ├── provider/         # LanguageProvider trait + ProviderRegistry
│   ├── rust/mod.rs       # Rust provider
│   ├── go/mod.rs         # Go provider
│   ├── python/mod.rs     # Python provider
│   ├── markdown/mod.rs   # Markdown provider
│   ├── json/mod.rs       # JSON provider
│   ├── toml/mod.rs       # TOML provider
│   ├── yaml/mod.rs       # YAML provider
│   ├── data.rs           # shared scanners for the data formats
│   └── mod.rs            # LanguageService facade
├── project/
│   ├── mod.rs            # Project detection + Workspace
│   └── file_tree.rs      # lazily loaded tree
├── search.rs             # project-wide text search
├── regex.rs              # a small regex engine for search
├── terminal/mod.rs       # init/restore, OSC 52 clipboard
└── ui/
    ├── theme.rs          # Mellow palette + semantic roles (single source of truth)
    ├── art.rs            # original ASCII art: mascot, wordmark, frames
    ├── header.rs         # identity bar / breadcrumb
    ├── tabs.rs           # tab pills
    ├── editor.rs         # gutter, syntax, cursorline, indent guides
    ├── file_tree.rs      # project sidebar
    ├── status_bar.rs     # the Mellow statusline
    ├── overlay.rs        # palette, quick open, prompts, search bar
    ├── welcome.rs        # adaptive welcome scene
    └── mod.rs            # layout + entry point
```

### Dependency direction

- `editor`, `filesystem`, `git`, `commands` **must not** depend on `ui` or `app`.
- `language` depends on nothing UI-related.
- `ui` may read `app` state and editor/language data, but must not contain
  editing logic.
- `app` is the conductor and may use everything.

The editor core must remain language-agnostic.

### Visual identity

Koda's look is a first-class concern, not an afterthought.

- **Mellow is the colour source of truth.** The palette and semantic mappings
  come from **Mellow**, the separately named colorscheme shipped with Helix
  (not Helix's default theme), defined in
  `runtime/themes/mellow.toml`.
  `src/ui/theme.rs` is the single point of truth: the raw palette in `palette`,
  semantic roles (`ACCENT`, `TEXT`, `PANEL_BG`, …) and `token_style(kind)` for
  syntax. Do not hard-code colours in widgets.
- **Koda's design is its own.** Mellow is the colour language; the layout,
  header, tabs, sidebar, statusline, popups and welcome scene are Koda's. Never
  copy Helix's or VS Code's UI.
- **Transparency is mandatory.** Never paint a full-screen background. Plain
  surfaces leave the background `Color::Reset` so the terminal's own background
  shows through. Only the statusline and popups use a `PANEL_BG`.
- **Art has a vocabulary.** Recurring motifs: `✦` (star), `☾` (moon), `❯`
  (pointer), `·` (separator), and the Koda familiar, a little star-cat. Reuse
  them consistently so the interface reads as one piece.
- **Art with restraint.** ASCII art belongs in the welcome scene and empty
  states, never behind code. When code is open, the editor must dominate.
- **Keep `ui/art.rs` clean.** Compositions are original, symmetric and padded to
  equal width so per-line centering keeps them aligned. The welcome screens are
  budget-aware and must never clip or overflow.

### Language detection vs. language support

These are separate concerns and must stay separate:

- **Detection** (`language/detection`) consumes lightweight
  `LanguageDescriptor`s and combines signals into a `DetectionResult`
  (`language`, `confidence`, `reasons`). It never imports provider
  implementations.
- **Providers** (`language/provider` + `language/{rust,go,python,markdown,json,toml,yaml}`)
  implement the `LanguageProvider` trait and *produce* those descriptors via
  `descriptor()`.

To add a language:

1. Create `language/<name>/mod.rs` implementing `LanguageProvider`.
2. Register it in `ProviderRegistry::builtin()`.
3. That's it — detection and highlighting work automatically.

Adding a language must never require touching `editor` or `ui`.

### Language detection signals (scored)

| Signal | Weight | Notes |
| --- | --- | --- |
| Special file name | 55 | e.g. `Dockerfile` |
| Shebang | 50 | `#!/usr/bin/env …` |
| Project context | +40 | corroboration only, not a standalone decision |
| File extension | 35 | one signal among many |
| Content hints | 20 each (max 3) | lightweight, no big parser |

File-level signals (name, shebang, extension, content) decide the language.
Project markers provide a **corroborating bonus**: `Cargo.toml` plus a `.rs`
extension yields *high* confidence Rust, but `Cargo.toml` alone never turns
`README.md` into Rust. A project marker says what the project is, not what a
given file is; the workspace tracks project kind separately.

Confidence is derived from the winning score and the margin to the runner-up.
If two languages are nearly tied, confidence is downgraded rather than guessed.

---

## 3. Coding conventions

- Rust edition **2024**, stable toolchain.
- Run `cargo fmt` and `cargo clippy --all-targets` before finishing. Clippy must
  be warning-free.
- Prefer clear, small modules over giant files. Keep methods focused.
- Use `Result`/`Option` and propagate errors; do not `unwrap()` on user input or
  filesystem operations (tests and proven invariants are fine).
- Comments explain *why*, not *what*. Keep them accurate — stale comments are
  worse than none.
- Public items get doc comments explaining intent.
- UI strings use plain ASCII except intentional glyphs (`▸ ▾ │ ● ⎇ ❯`).
- Measure positions in **characters**, not bytes, so non-ASCII text is correct.

---

## 4. Important design decisions

- **Rope-backed buffers** (`ropey`). Efficient edits and slicing on large files.
- **Operation-based undo.** Each edit records `(start, removed, inserted,
  cursor_before, cursor_after)`. Cheap and exact.
- **Line-based highlight cache.** Providers return `(spans, next_state)` so
  multi-line constructs (block comments) work. The cache is invalidated from the
  edited line down.
- **Lazy, `.gitignore`-aware file tree.** Directory listings are read only when
  expanded; ignored directories (`target`, `node_modules`, …) are skipped, and
  the project's `.gitignore` rules (including nested files and
  `.git/info/exclude`) hide generated and local files from the tree, quick open,
  the inline filter and workspace symbol search. Toggling hidden files reveals
  them again.
- **Git via subprocess.** We shell out to `git` instead of linking a heavy
  library. Failures degrade gracefully (no repo → no git UI).
- **OSC 52 clipboard.** Copy works over SSH and in most terminals; internal
  clipboard is the fallback and bracketed paste handles system paste.
- **Overlays are centered and cleared** with `Clear` before drawing so they are
  readable over the editor.
- **Transient status messages** expire so the language/git summary returns.
- **Centralised Mellow theme + art.** All colour flows through `ui/theme.rs`;
  all ASCII personality through `ui/art.rs`. Widgets render semantic roles, not
  literal colours. This keeps Koda recognisable and easy to evolve.

---

## 5. Development workflow

Work commit-by-commit. Each meaningful commit should:

1. Have a single clear purpose.
2. Compile (`cargo check`).
3. Pass tests (`cargo test`).
4. Be free of clippy warnings.
5. Leave the repository usable.

Before changing code:

1. Read the existing modules you will touch.
2. Understand the surrounding conventions.
3. Keep changes focused; avoid speculative rewrites.
4. Update `CHANGELOG.md` for user-visible changes and `ROADMAP.md` when
   direction changes.

Never break existing tests to make progress. Add tests for new behaviour:

- Editor and buffer logic: unit tests next to the code.
- Detection and providers: unit tests in their modules.
- UI composition: `tests/render.rs` using ratatui's `TestBackend`.

---

## 6. Rules for AI sessions

- Inspect before changing. Do not blindly rewrite working code.
- Prefer incremental progress over large speculative changes.
- When adding a capability, add it through the abstraction (providers,
  commands), not as a special case in the UI.
- Keep the command palette as the discoverability surface; add a `Command`
  entry for new user-facing actions.
- Preserve the zero-configuration promise. If a new feature can be made to
  "just work", do that; do not require the user to edit a config file.
- Keep documentation in sync: `README.md`, this file, `CHANGELOG.md`,
  `ROADMAP.md`.
- If unsure about a trade-off, favour the option that keeps the editor core
  clean and the UI minimal.

---

## 7. Testing and commands

```bash
cargo check                     # fast compile check
cargo test                      # all tests
cargo clippy --all-targets      # lints (must be clean)
cargo fmt                       # formatting
cargo run -- .                  # run against this project
cargo run -- src/main.rs        # run against a file
```
