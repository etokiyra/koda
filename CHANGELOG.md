# Changelog

All notable changes to Koda are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Initial editor foundation:
  - Rope-backed buffers with `ropey`.
  - Cursor movement (characters, words, lines, document, page), selection, and
    `Shift`-based selection.
  - Insertion, deletion, backspace/delete, auto-indent on newline, and
    "typing replaces selection".
  - Operation-based undo/redo.
- Multi-document tabs with dirty indicators; next/previous/close tab.
- Project and workspace abstraction with intelligent root detection
  (`Cargo.toml`, `go.mod`, `.git`).
- Lazy, git-aware file tree sidebar.
- Language subsystem:
  - Confidence-based detection engine combining project markers, extensions,
    file names, shebangs and content signals. Project markers corroborate a
    file's own signals rather than overriding them, so `README.md` in a Rust
    project stays plain text.
  - `LanguageProvider` trait and provider registry.
  - Rust and Go providers with syntax highlighting.
- User interface:
  - Header, tab strip, editor pane with gutter and line numbers, status bar.
  - Transparent-friendly colour scheme (no opaque full-screen background).
  - Command palette (`Ctrl+Shift+P`) and quick open (`Ctrl+P`).
  - Find and replace bar, go-to-line prompt, open-file prompt.
  - Search match highlighting with a current-match indicator.
- Command registry with stable ids, palette labels and shortcuts.
- Git integration: branch and per-file status via subprocess.
- Terminal integration: alternate screen, bracketed paste, OSC 52 clipboard.
- Documentation: `README.md`, `AGENTS.md`, `CHANGELOG.md`, `ROADMAP.md`.
- Tests: unit tests for editor, detection, providers, git parsing, base64, and
  rendering tests using ratatui's `TestBackend`.

### Keyboard shortcuts

- `Ctrl+S` save, `Ctrl+Q` quit, `Ctrl+O` open, `Ctrl+P` quick open,
  `Ctrl+Shift+P` command palette, `Ctrl+F` find, `Ctrl+H` replace,
  `Ctrl+G` go to line, `Ctrl+Z`/`Ctrl+Shift+Z` undo/redo, `Ctrl+A` select all,
  `Ctrl+C`/`Ctrl+X`/`Ctrl+V` clipboard, `Ctrl+B` toggle file tree,
  `Ctrl+W` close tab, `Ctrl+Tab`/`Ctrl+Shift+Tab` tabs, `Ctrl+/` toggle comment.

[Unreleased]: https://example.com/koda/compare/main...HEAD
