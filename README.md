# Koda

> A modern, lightweight, terminal-native IDE.

Koda is not "another terminal text editor". It is a development environment that
happens to live in your terminal: fast, minimal, keyboard-first and — above all —
**zero configuration**.

Install Koda, open a project, start coding.

```
 ✦ koda  ·  Rust  ·  src/main.rs
 ✦ files ────────────│ ▏ main.rs
▏ ▸ src             ? │ 1 fn main() {
  ·  main.rs         │ 2     println!("hi");
  ·  Cargo.toml      │ 3 }
 Rust  ☾ main 1±  ·  saved                Ln 1, Col 1  ☾ 5%
```

Koda is a tiny place to live while you code: Mellow colours, a little star-cat
mascot, and a lot of care — with the editor still first.

## Philosophy

- **Zero configuration.** Supported languages work out of the box. No LSP
  setup, no plugin marketplace, no config file required.
- **Koda owns language intelligence.** Detection and language support are
  first-class subsystems, not an afterthought.
- **Terminal-native.** Koda cooperates with the terminal. It never paints an
  opaque background, so transparent terminals keep working.
- **Keyboard-first.** Familiar shortcuts, plus a command palette for everything
  else.
- **Performance is a feature.** A rope-backed editor, lazy file tree and a
  responsive event loop.

## Design & personality

Koda's colour language is **Mellow**, the separate named colorscheme shipped
with Helix — not Helix's default theme. Koda takes Mellow's palette and its
semantic mappings — blue keywords, bright-blue types, green strings, magenta
numbers, grey italic comments, pink constants, yellow operators — and adapts
them to its own terminal-native interface. Koda is its own design, though: its
own header, tabs, sidebar, statusline and welcome scene.

Mellow is maintained upstream in Helix:
<https://github.com/helix-editor/helix/blob/master/runtime/themes/mellow.toml>.

Colour and personality live in one place — `src/ui/theme.rs` for the palette and
`src/ui/art.rs` for the ASCII art — so the whole environment stays coherent and
easy to evolve.

A few principles:

- **Transparency first.** Koda never paints a full-screen background. Plain
  surfaces use the terminal's own background, so transparency, blur and
  wallpapers show through. Only small, deliberate surfaces (the statusline,
  popups and the current line) get a Mellow panel background.
- **Art with restraint.** The Koda familiar, a little star-cat, appears where
  there is room for personality — the welcome scene and empty states — never
  behind your code.
- **One vocabulary.** A four-pointed star `✦`, a crescent moon `☾`, a `❯`
  pointer and `·` separators recur everywhere, so the interface feels like one
  piece.

## Status

Koda is under active, incremental development. Today it is a real editor with a
solid foundation; language intelligence is layered on top next.

Implemented:

- A rope-backed text editor with cursor, selection, undo/redo and auto-indent.
- Multi-file tabs, a lazy project file tree and git status indicators.
- Syntax highlighting for **Rust** and **Go** via language providers.
- A contextual, confidence-based language **detection engine** that combines
  project markers, extensions, file names, shebangs and content signals.
- A command palette, quick open, find/replace, go-to-line and file open.
- IDE editing behaviour: grouped undo, auto-pairing, smart newline,
  selection-aware indent/outdent, line move/duplicate and bracket matching.
- An inline fuzzy file filter in the sidebar, tab overflow scrolling and
  name disambiguation.
- Provider-driven diagnostics (lexical structural checks for Rust and Go today)
  surfaced as gutter markers, underlines, a statusline count, and `F8`/`Shift+F8`
  navigation.
- A document-symbol outline (`Ctrl+Shift+O`) for Rust and Go, with fuzzy
  filtering and jump-to-symbol, plus within-file go-to-definition (`F12`) and
  find-references (`Shift+F12`).
- Completion (`Ctrl+Space`) merging provider keywords with buffer identifiers.
- Formatting (`Ctrl+Shift+I`) through the language's own tool (`rustfmt`,
  `gofmt`), run off the UI thread, with missing tools reported up front.
- Hover (`Ctrl+Shift+H`) showing a symbol's kind, definition line and usage
  count.
- Language-tool discovery: **Language Setup…** reports which of
  `rust-analyzer`, `gopls`, `rustfmt` and `gofmt` Koda found, with install
  hints for the rest.
- An asynchronous **language-server client**: when `rust-analyzer` or `gopls` is
  installed Koda starts it for the workspace and shows its diagnostics, falling
  back to the built-in providers when no server is available.
- Project-wide symbol search (`Ctrl+T`), scanned off the UI thread.
- A keyboard-shortcuts cheatsheet (`F1`), a scroll margin, a live busy spinner
  and a gently animated mascot.
- Lightweight git integration (branch + per-file status, via the `git` binary).
- A distinctive Mellow-based visual identity: a semantic theme layer, an
  adaptive ASCII welcome scene, the Koda familiar and personality-rich empty
  states — all transparency-friendly.
- A background worker keeps language detection and git off the UI thread, and
  Koda repaints only when something changes.
- A clean provider/registry abstraction that makes adding a language
  straightforward.

## Installation

Koda is a standard Cargo project.

```bash
git clone <your-fork-or-repo> koda
cd koda
cargo build --release
# the binary is target/release/koda
```

During development:

```bash
cargo run -- .
cargo run -- path/to/file.rs
```

## Usage

```bash
koda              # open the current directory as a workspace
koda .            # same, explicitly
koda src/main.rs  # open a file inside its detected project
```

Koda detects the project root (`Cargo.toml`, `go.mod`, or a `.git` directory)
and establishes language context automatically.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+S` | Save |
| `Ctrl+Q` | Quit (press twice if there are unsaved changes) |
| `Ctrl+O` | Open file (path prompt) |
| `Ctrl+P` | Quick open |
| `Ctrl+Shift+P` | Command palette |
| `F1` | Keyboard-shortcuts cheatsheet |
| `Ctrl+F` | Find |
| `Ctrl+H` | Replace |
| `Ctrl+G` | Go to line |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Ctrl+Space` | Complete the word being typed |
| `Ctrl+Shift+I` | Format the active file |
| `Ctrl+Shift+H` | Hover: info about the symbol under the cursor |
| `Ctrl+A` | Select all |
| `Ctrl+B` | Toggle file tree |
| `Ctrl+E` | Focus file tree / editor |
| `Ctrl+W` | Close tab (press twice to discard unsaved changes) |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+/` | Toggle comment |
| `F8` / `Shift+F8` | Next / previous diagnostic |
| `Ctrl+Shift+O` | Go to symbol in the active file |
| `Ctrl+T` | Go to symbol in the workspace |
| `F12` / `Shift+F12` | Go to definition / find references |
| `Tab` / `Shift+Tab` | Indent / outdent selection |
| `Alt+↑` / `Alt+↓` | Move line up / down |
| `Ctrl+Shift+D` | Duplicate line |
| `/` (in the tree) | Filter project files |
| `.` (in the tree) | Toggle hidden files |

Editor keys behave as you would expect: arrows, `Home`/`End`, `PageUp`/`PageDown`,
`Shift`+arrows to select, `Ctrl`+arrows for word movement. Typing with a
selection replaces it, brackets and quotes pair up, and consecutive typing
undoes as one step.

## Architecture

```
src/
├── app/          # state, event loop, overlays, command dispatch
├── commands/     # the extensible command registry
├── editor/       # buffers, documents, cursor, history (no language logic)
├── filesystem/   # thin, well-behaved fs helpers
├── git/          # lightweight git integration
├── language/
│   ├── detection/# signals, scoring, confidence
│   ├── provider/ # the LanguageProvider trait + registry
│   ├── rust/     # Rust provider
│   └── go/       # Go provider
├── project/      # workspace, project detection, file tree
├── terminal/     # terminal lifecycle + OSC 52 clipboard
├── background.rs # worker thread for detection, git, and future language work
└── ui/           # rendering for every surface
```

The guiding rule: **language-specific logic never leaks into the editor or UI**.
Detection answers *what is this file*; providers answer *how do we support it*.
Expensive work runs on a background worker so the UI never blocks on it.

See [`AGENTS.md`](AGENTS.md) for the full architecture and conventions.

## Language detection

Extensions are only one signal. File-level signals decide *what a file is*;
project context corroborates them:

1. **File name** — special names like `Makefile`, `Dockerfile`
2. **Shebang** — `#!/usr/bin/env …`
3. **File extension** — `.rs`, `.go`, …
4. **Content analysis** — distinctive syntax, kept lightweight
5. **Project context** — `Cargo.toml`, `go.mod`, … (a confidence boost when it
   agrees with the file's own signals)

A project marker never overrides a file's own nature: `README.md` inside a Rust
project stays plain text, while `main.rs` inside that project becomes *high*
confidence Rust. The workspace separately tracks the project kind, so Koda still
understands that the file lives inside a Rust project.

Signals are scored and turned into a [`Confidence`](src/language/detection/confidence.rs).
The architecture allows Koda to eventually ask the user when detection is
ambiguous rather than silently guessing.

## Development

```bash
cargo test        # unit + rendering tests
cargo clippy      # lints
cargo fmt         # formatting
```

Contributions are developed commit-by-commit: small, focused, compiling and
tested. See [`ROADMAP.md`](ROADMAP.md) for where this is going and
[`CHANGELOG.md`](CHANGELOG.md) for what has changed.

## License

MIT.
