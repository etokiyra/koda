# Changelog

All notable changes to Koda are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed

- Corrected the theme to use the **actual upstream Helix Mellow colorscheme**.
  The earlier palette was a guess and was mistakenly described as Helix's
  default theme. The palette, syntax scopes, UI surfaces and diagnostic
  severities now follow
  `runtime/themes/mellow.toml`: blue keywords, bright-blue types, green strings,
  magenta numbers, grey italic comments, pink constants, yellow operators,
  bright-cyan macros, and the neutral `gray01`–`gray07` surfaces. Matching
  brackets now use Mellow's `ui.cursor.match` (yellow, bold, underlined)
  instead of a custom background. Documentation was corrected to match.

### Language intelligence

- Added provider-driven diagnostics. Rust and Go now perform a lexical
  structural check (unbalanced brackets, ignoring strings and comments) and
  declare the `Diagnostics` capability.
- Diagnostics are computed on the background worker: editing clears stale
  markers immediately and a fresh pass runs about 150 ms after typing pauses, so
  results are always for the current text and never block the keystroke.
- Problems surface as a gutter marker (error `●`, warning `▲`), an underline on
  the affected characters, a statusline count (`2✖ 1⚠`), and — when the cursor
  rests on one — its message in the statusline.
- New commands: **Next Diagnostic** (`F8`), **Previous Diagnostic**
  (`Shift+F8`) and **Show Diagnostics**, which lists every problem across open
  files and jumps to the chosen one.
- Added document symbols. **Go to Symbol…** (`Ctrl+Shift+O`) lists the Rust and
  Go definitions in the active file (functions, methods, structs, enums, traits,
  interfaces, modules, types, constants and macros) and jumps to the chosen one.
  Extraction is a lightweight provider scan; language-server symbols can replace
  it later without changing the UI.
- Go to definition (`F12`) resolves the word under the cursor to its definition
  in the same file, and Find References (`Shift+F12`) lists every occurrence in
  the file. Providers own the resolution; cross-file resolution comes with the
  language-server backends.
- Added completion. **Complete** (`Ctrl+Space`) opens a compact popup that merges
  the provider's keywords, types and builtins with identifiers already in the
  buffer. It filters as you type, navigates with the arrows and accepts with
  `Enter`/`Tab` or dismisses with `Esc`. Buffer completion works for any
  language; language servers can supply richer candidates later.
- Added formatting. **Format Document** (`Ctrl+Shift+I`) runs the language's
  trusted formatter (`rustfmt` for Rust, `gofmt` for Go) on a buffer snapshot
  through the background worker, then replaces the buffer as a single undoable
  edit. Koda reads the Rust edition from the nearest `Cargo.toml`. The tools are
  reused from the system; if one is missing Koda says exactly what to install
  instead of failing silently. Automatic tool provisioning is not implemented
  yet.
- Added hover. **Hover** (`Ctrl+Shift+H`) opens a dismissible popup anchored to
  the cursor showing what the word is: its definition kind and source line when
  it is defined in the file, plus how many times it occurs. The provider owns
  the content, so language-server hover can replace it later.
- Added project-wide symbol search. **Go to Symbol in Workspace…** (`Ctrl+T`)
  scans the project on the background worker and lists every Rust/Go definition
  in a filterable picker; choosing one opens its file and jumps to it. Files are
  mapped by extension (no content reads) and oversized files are skipped.
- Koda now checks whether a language's formatter is installed and marks
  **Format Document** unavailable in the palette with the reason
  ("rustfmt is not installed") before you try, rather than only failing on
  invocation.

### Performance

- Added a background worker thread. Language detection and git status now run
  off the UI thread and their results are applied as they arrive, so opening a
  file, saving, or starting Koda never waits on `git status` or disk reads.
  This is the foundation for asynchronous language intelligence (diagnostics,
  completion, and so on).
- Koda repaints only when input arrives, background work completes, a status
  message expires, or the terminal is resized.

### Polish & motion

- The editor keeps a small scroll margin (scrolloff), so the cursor never sits
  glued to the top or bottom edge and there is always context in view.
- Background work shows a live spinning sparkle in the statusline
  (`formatting…`, `searching symbols…`). It runs only while work is in progress.
- A keyboard-shortcuts cheatsheet (`F1`) is generated from the command registry,
  so it can never drift from the real keymap. It scrolls and shows the Koda
  familiar, which also blinks gently on the welcome screen.
- The statusline reports the active file's line ending (`LF`/`CRLF`) on wide
  terminals.
- Overlays that filter to nothing now say "no matches" instead of showing an
  empty panel.
- Animations only run when there is something to show (the welcome scene or
  in-progress work); an idle editor still does no work.

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
- The statusline shows the size of the current selection (characters or lines).
- Find searches line by line instead of copying the whole file on every
  keystroke, and Koda now only repaints when something actually changes.

### Visual identity (Mellow)

- Replaced the placeholder palette with the **Mellow** colour language — the
  separate named colorscheme shipped with Helix, not Helix's default — mapped
  semantically from its syntax scopes: blue keywords, bright-blue types, green
  strings, magenta numbers, grey italic comments, pink constants and yellow
  operators.
- Added `src/ui/theme.rs` as the single source of truth for palette and semantic
  roles, and `src/ui/art.rs` for original ASCII art.
- Introduced the **Koda familiar** — a little star-cat — with sleeping, awake and
  celebrating poses, plus a four-pointed star `✦`, crescent moon `☾`, `❯` pointer
  and `·` separator as a recurring visual vocabulary.
- A new adaptive, vertically-centred **welcome scene**: starfield, a tiny code
  window, the mascot, the `K O D A` wordmark, shortcuts and project context. It
  budgets space and degrades gracefully on small or narrow terminals.
- Redesigned every surface: a breadcrumb header, tab pills, a hairline sidebar
  rule with right-aligned git state, a subtle cursorline, indent guides, a panel
  statusline with a language pill, and Mellow-styled popups.
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
