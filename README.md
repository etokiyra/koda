# Koda

> A modern, lightweight, terminal-native IDE.

Koda is not "another terminal text editor". It is a development environment that
happens to live in your terminal: fast, minimal, keyboard-first and — above all —
**zero configuration**.

Install Koda, open a project, start coding.

```
╭──────────────────────────────────────────────────────────────╮
│ koda  Rust                                          main.rs │
├────────────┬─────────────────────────────────────────────────┤
│ PROJECT    │                                                 │
│ ▾ src/     │  fn main() {                                    │
│     main.rs│      println!("Hello, world!");                 │
│   Cargo.toml│  }                                             │
├────────────┴─────────────────────────────────────────────────┤
│ ● Rust  │  ⎇ main 1±                    Ln 3, Col 5          │
╰──────────────────────────────────────────────────────────────╯
```

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
- Lightweight git integration (branch + per-file status, via the `git` binary).
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
| `Ctrl+F` | Find |
| `Ctrl+H` | Replace |
| `Ctrl+G` | Go to line |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Ctrl+A` | Select all |
| `Ctrl+B` | Toggle file tree |
| `Ctrl+W` | Close tab |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+/` | Toggle comment |

Editor keys behave as you would expect: arrows, `Home`/`End`, `PageUp`/`PageDown`,
`Shift`+arrows to select, `Ctrl`+arrows for word movement, and `Tab` inserts
indentation. Typing with a selection replaces it.

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
└── ui/           # rendering for every surface
```

The guiding rule: **language-specific logic never leaks into the editor or UI**.
Detection answers *what is this file*; providers answer *how do we support it*.

See [`AGENTS.md`](AGENTS.md) for the full architecture and conventions.

## Language detection

Extensions are only one signal. Detection combines:

1. **Project metadata** — `Cargo.toml`, `go.mod`, … (strongest signal)
2. **Project context** — a file living inside a known project
3. **File extension** — `.rs`, `.go`, …
4. **Special file names** — `Makefile`, `Dockerfile`, … (extensible)
5. **Shebangs** — `#!/usr/bin/env python`
6. **Content analysis** — distinctive syntax, kept lightweight

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
