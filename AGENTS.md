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
│   ├── layout.rs         # tab expansion + soft-wrap segments
│   ├── position.rs       # Position / Selection / Cursor
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
│   ├── shell/mod.rs      # Shell provider
│   ├── web/mod.rs        # TypeScript/JavaScript provider
│   ├── c/mod.rs          # C/C++ provider
│   ├── java/mod.rs       # Java provider
│   ├── csharp/mod.rs     # C# provider
│   ├── php/mod.rs        # PHP provider
│   ├── kotlin/mod.rs     # Kotlin provider
│   ├── sql/mod.rs        # SQL provider
│   ├── ruby/mod.rs       # Ruby provider
│   ├── perl/mod.rs       # Perl provider
│   ├── asm/mod.rs        # Assembly provider
│   ├── dart/mod.rs       # Dart provider
│   ├── elixir/mod.rs     # Elixir provider
│   ├── swift/mod.rs      # Swift provider
│   ├── html/mod.rs       # HTML provider
│   ├── css/mod.rs        # CSS provider
│   ├── lua/mod.rs        # Lua provider
│   ├── markdown/mod.rs   # Markdown provider
│   ├── json/mod.rs       # JSON provider
│   ├── toml/mod.rs       # TOML provider
│   ├── yaml/mod.rs       # YAML provider
│   ├── data.rs           # shared scanners for the data formats
│   └── mod.rs            # LanguageService facade
├── project/
│   ├── mod.rs            # Project detection + Workspace
│   ├── create.rs         # deterministic, offline project templates
│   └── file_tree.rs      # lazily loaded tree
├── recent.rs             # recent projects and files for the welcome screen
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
    ├── overlay.rs        # palette, quick open, prompts, directory picker, new-project flow
    ├── welcome.rs        # the welcome home screen
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
- **Providers** (`language/provider` + `language/{rust,go,python,shell,web,c,java,csharp,php,kotlin,sql,ruby,perl,asm,dart,elixir,swift,html,css,lua,markdown,json,toml,yaml}`)
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
- **Welcome-first startup.** Koda opens on the welcome home screen. A path from
  the command line is offered as an action, a saved session as a resume action,
  and recent projects/files are loaded from the state directory; nothing is
  opened until the user chooses. Session saving is gated on the user engaging
  with a project, so an untouched session is never overwritten.
- **Deterministic project scaffolding.** `project/create.rs` writes conventional
  project files directly instead of invoking `cargo`, `go` or `pip`, so
  creation is instant, offline and toolchain-independent. It never deletes a
  partial project: a failure returns the root so the UI can explain what
  happened.
- **User-local tool installs.** Provisioning never writes to a system-owned
  location: `rustup`/`go install` place tools in the user's home, Python uses a
  virtualenv Koda manages under its data directory (which seeds its own `pip`,
  so a Python without `pip` or one marked externally managed still works), with
  `--user`/`pipx`/`uv` as fallbacks, and npm-based tools use a prefix Koda
  manages under its data directory (with a matching cache). A missing
  permission therefore cannot make an install fail, and every resulting bin
  directory is searched when locating tools, alongside the usual user bin
  directories.
- **Verified install attempts.** A tool's provisioning is an ordered list of
  self-contained steps — run a command, download an archive (optionally
  verifying a published SHA-256), fetch and verify an Adoptium JDK, or extract.
  Koda re-probes the tool after every attempt rather than trusting a package
  manager's exit code, and reports a missing prerequisite (a whole absent
  toolchain) instead of offering an install that cannot run. Installs into the
  shared managed directory are guarded by an advisory lock (with stale-lock
  recovery) so concurrent Koda instances cannot corrupt the same npm prefix or
  virtualenv.
- **Managed runtimes.** Some language servers need a runtime, so Koda provisions
  one under its own data directory and launches the server with it: the .NET SDK
  for OmniSharp (`DOTNET_ROOT`), and a checksum-verified Eclipse Adoptium JDK for
  Eclipse JDT (`JAVA_HOME`). `Tool::launch_env` supplies the environment for both
  probing and launching; the user's system environment is never modified.
- **Split editor.** The editor keeps a single active document; a split stores
  one document index per pane and `editor.active` follows the focused pane, so
  every existing editing path keeps working unchanged. Panes share the tab
  strip, and closing a pane's file collapses the split.
- **One language server per language.** The app tracks servers in a map keyed by
  `LanguageId`; each has its own handshake, restart budget and failure state.
  Feature requests resolve against the active document's language, and a
  language's server failing never disturbs another. Workspace-wide requests
  prefer the active language's server, then any ready one.
- **Inferred indentation.** Each document detects its indentation unit from the
  file's leading whitespace (falling back to four), and uses it for
  auto-indent, `Tab` and indent/outdent. This is zero-configuration and keeps
  the editor core language-agnostic: the width is a property of the file, not a
  per-language branch.
- **Multiple cursors are a small, explicit model.** The primary stays on
  `Document` as `cursor`/`selection`; `Document::cursors` holds the secondary
  `Cursor`s (anchor + cursor), kept sorted, unique and distinct from the
  primary. Edits are planned per cursor against the pre-edit text and applied
  bottom-up, then recorded as one `Edit` whose `ops` list makes the whole
  keystroke a single undo step. `apply_multi_edit` is the only path that mutates
  several ranges, and it moves cursors even for a zero-op keystroke so bracket
  skip-over works everywhere. Navigation deliberately collapses to the primary;
  every other single-point edit clears the extras so a stale cursor can never
  survive an unrelated operation.
- **Soft wrap is a display-only transform with one source of truth.** A position
  is always a character position; wrapping only adds a derived *visual row*.
  `editor/layout.rs` owns both the character ↔ display-column mapping (tabs
  expand there) and the wrap boundaries, and the renderer and the editor's
  Up/Down movement both call it, so they cannot disagree. The viewport is
  `scroll_top` (a logical row) plus `scroll_subline` (which visual row of it is
  on screen); the cursor's logical line is always kept at or above the top, which
  bounds scrolling work to the visible rows instead of scanning the document.
  Wrapping never edits the buffer, so undo/redo, diagnostics, LSP positions and
  multiple cursors are unaffected; turning it off restores horizontal scrolling.
- **Formatting reuses trusted tools through one pipeline.** A provider declares
  the `Formatting` capability and implements `format()` by calling a shared
  helper in `language/format.rs` (`rustfmt`, `gofmt`, `prettier`,
  `clang-format`, `shfmt`, `perltidy`). Every helper feeds a buffer snapshot on
  stdin and takes stdout, so unsaved edits are formatted in place and the result
  applies as one undoable edit. The provider's `formatter()` names the
  executable, and `Tool::for_language(language, Formatter)` lets the palette
  explain a missing formatter before the user invokes it. Language-server
  formatting is preferred when the server advertises it.
- **The web tokenizer uses lookback heuristics, not a parser.** A `/` in
  expression position starts a regex literal (division after a value does not),
  and `<` opens a JSX tag only after an expression start, so `Array<Foo>` and
  `a < b` stay operators. This keeps the built-in highlighter predictable on
  incomplete and ambiguous code; it is a lexical aid, not a semantic parser.
- **Scenes are drawn on a canvas.** `ui/art.rs` composes each welcome scene by
  placing glyphs at coordinates on a small `Canvas`, then turning runs of equal
  style into spans. This keeps the art symmetric and lets one element animate
  without the rest drifting; every scene is padded to an equal width so
  per-line centering stays aligned. The welcome composer budgets its height and
  drops the scene for just the familiar, or nothing, on small terminals.
- **Motion is a toggle, not a config file.** `App::motion` (palette:
  **Toggle Animations**) freezes the scene and the busy sparkle. Zero
  configuration is preserved; reduced motion is one keypress away.
- **Saves are atomic and lossless.** Writes go to a sibling temporary file that
  is flushed and renamed over the original, preserving permissions. Loading
  refuses binary, non-UTF-8 and oversized files rather than replacing bytes with
  U+FFFD and writing them back, and filesystem rename/copy never overwrite an
  existing destination.
- **Git is parsed losslessly.** `git status` is read as `--porcelain -z` raw
  bytes, so leading status columns, spaces, non-ASCII names and renames survive;
  staging is scoped to the workspace root.
- **Detection is deterministic and file-first.** Provider descriptors are sorted
  by language id before the engine is built, the content-hint weight is capped
  at exactly three, and the project-context bonus applies only to a language
  with a file-level signal (name, shebang or extension) — a project marker never
  overrides a file's own nature.
- **Editor history tracks a save point.** Each edit has a unique id and the
  document remembers the id at its save point, so undo/redo restore the clean
  state exactly; coalescing is broken when a document is saved.
- **Language servers are attached for the whole session.** A document detected
  after the handshake is sent `didOpen`, closing sends `didClose`, the restart
  budget is only forgiven after a stable uptime (so a crash-after-initialize
  loop is bounded), and one malformed message does not tear down a live
  connection. Results are still matched to their request id and document.
- **Provisioning is isolated and bounded.** Install subprocesses run from Koda's
  own tools directory (never the user's project, so a local `.npmrc` cannot
  hijack `npm`), downloads and commands have time and output limits, checksum
  verification fails closed, and the install lock records a nonce so a reclaimed
  holder cannot delete a successor's lock.
- **Adversarial input is bounded.** Search highlighting runs on the UI thread,
  so the regex engine carries a per-line step budget and the `.gitignore` glob
  matcher is polynomial; a crafted pattern degrades to "no match" instead of
  freezing the editor.
- **The LSP encoding boundary is centralized.** Koda's internal positions count
  Unicode scalar values. Everything crossing the LSP boundary is converted in
  `language/lsp/convert.rs` (`char_to_lsp`/`lsp_to_char`, and
  `edits_to_chars`) using the actual line text and the negotiated
  `PositionEncoding` (UTF-8, UTF-16 or UTF-32). Outbound requests convert the
  cursor; inbound diagnostics, edits, formatting and locations convert back
  before use. Conversion always clamps to a character boundary and never splits
  a code point. Do not add ad-hoc character/byte arithmetic in the UI.
- **Server responses are validated against the document they describe.** A
  document-sensitive request records `(language, id, path, buffer version)` at
  request time; a response is applied only if it is still the newest request and
  the document is open at the same version (navigation also requires it to be
  active). Code actions are re-validated when applied. Never apply an edit or
  move the cursor from a response that has been superseded.
- **Requests and subprocesses are bounded.** Every LSP request has a deadline
  (formatting a longer one); expired requests are removed and their UI state
  cleared. External processes run through `process::wait_captured`, which drains
  both pipes, caps output and kills the child at a deadline. New subprocess call
  sites must use it rather than `Command::output` or `wait_with_output`.
- **Background work is isolated.** A small worker pool runs background requests,
  so one slow operation cannot stall unrelated services. Automatic,
  superseding snapshots (diagnostics) may be dropped when the queue is
  saturated; user-initiated work must not be dropped.

### Known limitations

- **`.gitignore` discovery is capped.** At most 256 nested `.gitignore` files and
  4096 directories are read per project; in an enormous monorepo a deeper rule
  beyond the cap is not applied. The caps keep project open predictable.
- **Incremental sync is not implemented.** Koda always sends whole-document
  changes, which the protocol permits as the safe fallback even for servers that
  prefer incremental updates.
- **Non-UTF-8 paths in state.** Recent/session stores serialize paths lossily, so
  a non-UTF-8 path may not round-trip.

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
