# Koda

<p align="center"><em>A modern, lightweight, terminal-native IDE.</em></p>

<p align="center">
  <a href="LICENSE"><img alt="License: GPLv3" src="https://img.shields.io/badge/license-GPLv3-90b99f?style=flat-square"></a>
  <img alt="Rust 2024" src="https://img.shields.io/badge/rust-2024-ea83a5?style=flat-square&logo=rust&logoColor=white">
  <img alt="Tests" src="https://img.shields.io/badge/tests-227%20passing-9dc6ac?style=flat-square">
  <a href="https://github.com/etokiyra/koda/commits/master"><img alt="Last commit" src="https://img.shields.io/github/last-commit/etokiyra/koda?style=flat-square&color=e29eca"></a>
  <a href="https://github.com/etokiyra/koda/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/etokiyra/koda?style=flat-square&color=aca1cf"></a>
  <a href="https://github.com/etokiyra/koda/issues"><img alt="Issues" src="https://img.shields.io/github/issues/etokiyra/koda?style=flat-square&color=e6b99d"></a>
</p>

<p align="center">
  <img alt="Zero configuration" src="https://img.shields.io/badge/config-zero-ea83a5?style=flat-square">
  <img alt="Terminal native" src="https://img.shields.io/badge/terminal-native-aca1cf?style=flat-square">
  <a href="https://github.com/helix-editor/helix/blob/master/runtime/themes/mellow.toml"><img alt="Theme: Mellow" src="https://img.shields.io/badge/theme-Mellow-e29eca?style=flat-square"></a>
  <img alt="Made with Rust" src="https://img.shields.io/badge/made%20with-Rust-b9aeda?style=flat-square&logo=rust&logoColor=white">
  <a href="https://github.com/etokiyra/koda/issues"><img alt="PRs welcome" src="https://img.shields.io/badge/PRs-welcome-90b99f?style=flat-square"></a>
</p>

```text
    ✦ ───────────────────────────────────────── ☾

     _  _____  ____    _
    | |/ / _ \|  _ \  / \
    | ' / | | | | | |/ _ \
    | . \ |_| | |_| / ___ \
    |_|\_\___/|____/_/   \_\

    ✦   your cozy little coding space   ✦

                   /\___/\
                   ( ･ω･ )
                    > ω <
                   /|   |\

    ✦ ───────────────────────────────────────── ☾
```

Koda is not another terminal text editor. It is a small development environment
that happens to live in your terminal — fast, minimal, keyboard-first and,
above all, **zero configuration**.

> Install Koda. Open a project. Start coding.

No LSP config. No plugin marketplace. No dotfiles to hand-write. Koda detects the
language, brings its own intelligence, and lets you get on with the interesting
part.

```text
 ✦ koda  ·  Rust  ·  src/main.rs                                    ✧  ·  ✧
 ✦ files ────────────────────────────│ ▏ main.rs
 ▏ ▸ src                             │ ● 1  use std::io;
   ▸ tests                           │   2
   · Cargo.toml                      │   3  fn main() -> io::Result<()> {
   · CHANGELOG.md                    │   4      let name = "koda";
   · README.md                       │   5      println!("hello from {name}");
   · ROADMAP.md                      │   6      Ok(())
                                     │   7  }
 Rust  1✖  ☾ main 2±  ✧ lsp           LF   Ln 5, Col 16   ☾ 42%
```

---

## ✦ Why Koda

- **Zero configuration.** Supported languages work out of the box. Koda writes
  no config, asks for none, and needs none.
- **Koda owns language intelligence.** Detection, providers and the language
  server lifecycle are first-class subsystems — including fetching the tools
  you are missing, from trusted package managers, with one keystroke.
- **Terminal-native.** Koda cooperates with your terminal. It never paints an
  opaque background, so transparency, blur and wallpaper keep working.
- **Keyboard-first.** Familiar shortcuts for muscle memory, plus a command
  palette that makes everything discoverable.
- **Fast by design.** A rope-backed editor, a lazy file tree, background
  workers, and a UI that only repaints when something actually changes.

## ☾ Language intelligence

Koda layers real IDE features on top of a clean provider abstraction. When a
language server is available it takes over; when it is not, built-in providers
keep you productive **offline**.

- **Diagnostics** — gutter markers, underlines, a statusline count, `F8`/`Shift+F8`
  navigation, a diagnostics list, and an optional inline note at the end of the
  affected line (**Toggle Inline Diagnostics**).
- **Completion** — `Ctrl+Space`, instantly merging buffer identifiers, language
  keywords, and server candidates.
- **Hover** — `Ctrl+Shift+H`, with the symbol's kind, definition and usage count.
- **Navigation** — `F12` go-to-definition and `Shift+F12` find-references, across
  files when a server is attached.
- **Symbols** — `Ctrl+Shift+O` for the file, `Ctrl+T` for the whole workspace
  (from the language server when attached, with Koda's built-in scan as the
  instant, offline fallback).
- **Rename** — `F2`, applying a workspace edit across every affected file.
- **Code actions** — `Ctrl+.` for quick fixes and refactors.
- **Formatting** — `Ctrl+Shift+I` through the language's own tool (`rustfmt`,
  `gofmt`), never blocking the UI.

### Zero-configuration language support

The complexity lives inside Koda:

```text
   open a file ──▶ detect language ──▶ choose a provider
                        │
                        ├─▶ highlight · diagnostics · symbols · completion · hover
                        │
                        └─▶ if a language server is installed
                              start it · sync documents · route features through it
                              (and it stays out of your way when none exists)
```

Missing something? Koda notices when a Rust, Go or Python file is open without
its language server and offers to install it, once, without blocking startup.
**Language Setup…** in the command palette then lists every tool Koda knows
about and installs a missing one with a single `Enter` — through the official
channel, so provenance and integrity stay with the package manager:

| Tool | Purpose | Koda installs it with |
| --- | --- | --- |
| `rust-analyzer` | Rust language server | `rustup component add rust-analyzer` |
| `gopls` | Go language server | `go install golang.org/x/tools/gopls@latest` |
| `pylsp` | Python language server | `pipx install python-lsp-server` (or `pip`) |
| `rustfmt` | Rust formatting | `rustup component add rustfmt` |
| `gofmt` | Go formatting | ships with the Go toolchain |

Servers start lazily, recover automatically if they exit, and fall back to the
built-in providers whenever one is unavailable, so editing never depends on
them.

### Supported languages

| Language | Syntax | Diagnostics | Symbols | Completion | Hover | Navigation | Rename |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Rust** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **Go** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **Python** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **Markdown** | built-in | — | built-in (headings) | — | — | — | — |
| **JSON** | built-in | built-in | built-in (top-level keys) | built-in (literals) | — | — | — |
| **TOML** | built-in | built-in | built-in (tables & keys) | built-in (literals) | — | — | — |
| **YAML** | built-in | built-in | built-in (top-level keys) | built-in (literals) | — | — | — |

Prose and configuration files are first-class too. Markdown, JSON, TOML and YAML
get syntax highlighting, structural diagnostics where they make sense, and a
symbol outline, all offline and with no setup. **Python** works offline through
Koda's built-in intelligence (highlighting, diagnostics, symbols, completion,
hover, navigation) and gains rename and code actions when Koda installs
`pylsp` for it. Adding a language means implementing one trait and registering
it — no changes to the editor or the UI.

## ❯ Editing & workflow

The editor is the heart of Koda, and it is built for real projects.

- Multiple files in tabs, a lazy `.gitignore`-aware project tree, file
  create/rename/delete, git branch and per-file status, a changed-files list
  (`Ctrl+Shift+G`) and a **Commit Changes…** flow.
- A **split editor** (`Alt+V`): view two files side by side, with `Alt+O`
  moving focus between the panes. The focused pane owns the cursor and the
  active tab.
- Grouped undo, auto-pairing, smart newline, selection-aware indent/outdent,
  line move/duplicate, delete-line, go-to-matching-bracket, matching-bracket
  highlighting and a kill-ring with `Alt+Y` yank-pop.
- Quick open (`Ctrl+P`), inline fuzzy file filtering in the sidebar (`/`),
  find/replace with case, whole-word and regex options, project-wide text
  search (`Ctrl+Shift+F`), go-to-line, and **Revert File** to discard local
  edits.
- Project-wide symbol search, session persistence (open files, cursors and
  expanded folders come back next launch) and external-change detection.
- A background worker keeps detection, diagnostics, formatting and git off the
  UI thread.

## ✧ Look & feel

Koda has a personality, but the code always comes first.

- **The Mellow colours.** Koda speaks the palette and semantic mappings of
  [Mellow][mellow], the separate named colorscheme shipped with Helix — not
  Helix's default theme. Blue keywords, bright-blue types, green strings,
  magenta numbers, grey italic comments.
- **Transparency first.** Plain surfaces use your terminal's own background;
  only the statusline, popups and the current line carry a soft panel.
- **A little familiar.** A star-cat keeps you company on the welcome screen and
  in empty states — never behind your code.
- **One visual vocabulary.** `✦` stars, `☾` moons, `❯` pointers and `·`
  separators recur throughout, so the whole environment reads as one piece.

[mellow]: https://github.com/helix-editor/helix/blob/master/runtime/themes/mellow.toml

## ✦ Install

Koda is a standard Cargo project (Rust edition 2024).

```bash
git clone https://github.com/etokiyra/koda.git
cd koda
cargo build --release
# binary: target/release/koda
```

Or install it onto your `PATH`:

```bash
cargo install --path .
```

During development:

```bash
cargo run -- .                 # open the current directory
cargo run -- src/main.rs       # open a file inside its detected project
```

## ❯ Usage

```bash
koda              # open the current directory as a workspace
koda .            # same, explicitly
koda src/main.rs  # open a file inside its detected project
```

Koda finds the project root (`Cargo.toml`, `go.mod`, or a `.git` directory) and
establishes the language context automatically.

## ☾ Keyboard shortcuts

<details>
<summary>Expand the full keymap</summary>

| Shortcut | Action |
| --- | --- |
| `Ctrl+S` | Save |
| `Ctrl+Shift+S` | Save as |
| `Ctrl+Q` | Quit (press twice if there are unsaved changes) |
| `Ctrl+O` | Open file (path prompt) |
| `Ctrl+N` | New file |
| `Ctrl+P` | Quick open |
| `Ctrl+Shift+P` | Command palette |
| `F1` | Keyboard-shortcuts cheatsheet |
| `F5` | Refresh the file tree and git status |
| `Ctrl+F` / `Ctrl+H` | Find / replace |
| `F3` / `Shift+F3` | Find next / previous |
| `Ctrl+Shift+F` | Search in the whole project |
| `Alt+C` / `Alt+W` / `Alt+R` (in find) | Toggle case / whole word / regex |
| `Alt+Enter` (in replace) | Replace every match |
| `Ctrl+G` | Go to line |
| `Ctrl+Shift+G` | List the files changed in git |
| `Ctrl+Shift+M` | Show diagnostics |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Alt+Y` | Yank-pop: replace the last paste with an earlier kill |
| `Ctrl+Space` | Complete the word being typed |
| `Ctrl+Shift+I` | Format the active file |
| `Ctrl+Shift+H` | Hover: info about the symbol under the cursor |
| `Ctrl+A` | Select all |
| `Ctrl+B` | Toggle file tree |
| `Ctrl+E` | Focus file tree / editor |
| `Alt+V` | Split the editor into two panes |
| `Alt+O` | Focus the other pane |
| `Ctrl+W` | Close tab (press twice to discard unsaved changes) |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+PageDown` / `Ctrl+PageUp` | Next / previous tab |
| `Ctrl+/` | Toggle comment |
| `F8` / `Shift+F8` | Next / previous diagnostic |
| `Ctrl+Shift+O` | Go to symbol in the active file |
| `Ctrl+T` | Go to symbol in the workspace |
| `F12` / `Shift+F12` | Go to definition / find references |
| `F2` | Rename symbol |
| `Ctrl+.` | Code actions |
| `Tab` / `Shift+Tab` | Indent / outdent selection |
| `Alt+↑` / `Alt+↓` | Move line up / down |
| `Ctrl+Shift+D` | Duplicate line |
| `Ctrl+Shift+K` | Delete line |
| `Alt+M` | Go to matching bracket |
| `/` (in the tree) | Filter project files |
| `.` (in the tree) | Toggle hidden files |

Editor keys behave as you would expect: arrows, `Home`/`End`, `PageUp`/`PageDown`,
`Shift`+arrows to select, `Ctrl`+arrows for word movement. Typing with a
selection replaces it, brackets and quotes pair up, and consecutive typing
undoes as one step.

</details>

## ✦ Architecture

<details>
<summary>Expand the source tree</summary>

```text
src/
├── app/          # state, event loop, overlays, command dispatch
├── commands/     # the extensible command registry
├── editor/       # buffers, documents, cursor, history (no language logic)
├── filesystem/   # thin, well-behaved fs helpers
├── git/          # lightweight git integration
├── language/
│   ├── detection/  # signals, scoring, confidence
│   ├── provider/   # the LanguageProvider trait + registry
│   ├── lsp/        # JSON-RPC client, lifecycle, result conversion
│   ├── tools/      # discovery + trusted provisioning
│   ├── data.rs     # shared scanners for JSON/TOML/YAML
│   ├── rust/       # Rust provider
│   ├── go/          # Go provider
│   ├── python/     # Python provider (built-in, offline)
│   ├── markdown/   # Markdown provider (headings as symbols)
│   └── json|toml|yaml/  # configuration-data providers
├── project/      # workspace, project detection, file tree
├── regex.rs      # a small regex engine for search
├── search.rs     # project-wide text search
├── session.rs    # per-project session persistence
├── terminal/     # terminal lifecycle + OSC 52 clipboard
├── background.rs # worker thread for detection, LSP, git and formatting
└── ui/           # rendering for every surface
```

The guiding rule: **language-specific logic never leaks into the editor or the
UI.** Detection answers *what is this file*; providers answer *how do we support
it*. Everything expensive runs on a background worker so the UI never blocks.

</details>

## ☾ Language detection

Extensions are only one signal. File-level evidence decides *what a file is*;
project context corroborates it:

1. **File name** — special names like `Makefile`, `Dockerfile`
2. **Shebang** — `#!/usr/bin/env …`
3. **File extension** — `.rs`, `.go`, …
4. **Content analysis** — distinctive syntax, kept lightweight
5. **Project context** — `Cargo.toml`, `go.mod`, … (a confidence boost when it
   agrees with the file's own signals)

A project marker never overrides a file's own nature: `README.md` inside a Rust
project stays plain text, while `main.rs` becomes *high* confidence Rust — and
the workspace still understands that it lives inside a Rust project.

## ❯ Development

```bash
cargo test                    # unit + rendering tests
cargo clippy --all-targets    # lints (must be clean)
cargo fmt                     # formatting
cargo run -- .                # try it
```

Contributions are developed commit-by-commit: small, focused, compiling and
tested. See [`ROADMAP.md`](ROADMAP.md) for where this is going and
[`CHANGELOG.md`](CHANGELOG.md) for what has changed.

## ✦ Credits & license

- Colour language: **[Mellow][mellow]**, the named colorscheme shipped with
  [Helix](https://helix-editor.com), by Rohit K Viswanath. Koda translates its
  palette and semantic mappings into its own interface; the layout, components
  and ASCII art are Koda's.
- Built on the shoulders of [`ratatui`](https://ratatui.rs),
  [`crossterm`](https://github.com/crossterm-rs/crossterm),
  [`ropey`](https://github.com/cessen/ropey) and
  [`serde_json`](https://github.com/serde-rs/json).

Licensed under the [GNU General Public License v3.0](LICENSE).

<p align="center"><sub>✦ &nbsp; made with care &nbsp; ☾</sub></p>
