# Changelog

All notable changes to Koda are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Performance

- Added a background worker thread. Language detection and git status now run
  off the UI thread and their results are applied as they arrive, so opening a
  file, saving, or starting Koda never waits on `git status` or disk reads.
  This is the foundation for asynchronous language intelligence (diagnostics,
  completion, and so on).
- Koda repaints only when input arrives, background work completes, a status
  message expires, or the terminal is resized.

### Editing & UX

- Undo grouping: consecutive typing, backspacing and forward-deletes coalesce
  into one undo step; moving the cursor breaks the group.
- Auto-pairing for brackets and double quotes, with skip-over, empty-pair
  deletion and selection wrapping.
- Smart newline: Enter between an empty pair expands to an indented block, and
  indentation deepens after an opening bracket.
- Selection-aware Tab/Shift+Tab indent and outdent.
- Line operations: move line up/down and duplicate line.
- The command palette now shows a description for every command, marks
  unavailable commands with a reason (for example "not available for Rust",
  "nothing to undo") and keeps shortcuts right-aligned.
- New commands: Save All, Close All Tabs, Toggle Hidden Files, Indent, Outdent,
  Move Line Up/Down and Duplicate Line.
- Quick Open lists recently opened files first.
- Closing a modified tab now asks for confirmation instead of refusing.
- The tab strip scrolls around the active tab when tabs overflow, shows overflow
  chevrons, truncates long titles and disambiguates duplicate file names by
  parent folder.
- File-tree rows truncate long names with an ellipsis instead of spilling into
  the editor, and the tree header brightens when the tree has focus.
- Pressing `/` in the file tree opens an inline fuzzy filter over the project
  files (scored on paths relative to the project root), with keyboard
  navigation and Enter to open.
- Find prefills from a single-line selection and starts at the first match at
  or after the cursor instead of always jumping to the top of the file.
- Find searches line by line instead of copying the whole file on every
  keystroke, and Koda now only repaints when something actually changes.

### Visual identity (Mellow)

- Replaced the placeholder palette with the **Mellow** colour language (the
  default Helix theme), mapped semantically: keywords almond, types and
  functions white, strings silver, numbers chamois, comments sirocco, with lilac
  and lavender for operators, punctuation and structure.
- Added `src/ui/theme.rs` as the single source of truth for palette and semantic
  roles, and `src/ui/art.rs` for original ASCII art.
- Introduced the **Koda familiar** — a little star-cat — with sleeping, awake and
  celebrating poses, plus a four-pointed star `✦`, crescent moon `☾`, `❯` pointer
  and `·` separator as a recurring visual vocabulary.
- A new adaptive, vertically-centred **welcome scene**: starfield, a tiny code
  window, the mascot, the `K O D A` wordmark, shortcuts and project context. It
  budgets space and degrades gracefully on small or narrow terminals.
- Redesigned every surface: a breadcrumb header, tab pills, a hairline sidebar
  rule with right-aligned git state, a bossanova cursorline, comet indent
  guides, a revolver statusline with a language pill, and Mellow-styled popups.
- Transparency preserved: the editor and all plain surfaces keep the terminal's
  own background; only the statusline and popups use a Mellow panel background.
  A render test guards this.
- Added `examples/preview.rs`, a `TestBackend` harness for inspecting rendered
  screens as text.

### Added

- Initial editor foundation:
  - Rope-backed buffers with `ropey`.
  - Cursor movement (characters, words, lines, document, page), selection, and
    `Shift`-based selection.
  - Insertion, deletion, backspace/delete, auto-indent on newline, and
    "typing replaces selection".
  - Operation-based undo/redo.
- Matching-bracket highlighting (Mellow `ui.cursor.match`): nesting- and
  type-aware, and skips brackets inside comments and strings.
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
  `Ctrl+E` focus file tree, `Ctrl+W` close tab, `Ctrl+Tab`/`Ctrl+Shift+Tab`
  tabs, `Ctrl+/` toggle comment.

[Unreleased]: https://example.com/koda/compare/main...HEAD
