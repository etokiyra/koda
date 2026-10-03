# Roadmap

Koda is developed incrementally. This roadmap describes direction, not promises.
Items move as they become well-defined; nothing here is a commitment to a date.

The guiding goal:

> Install Koda. Run `koda .`. Open a file. Start coding. It just works.

---

## Guiding sequence

```
editor foundation  →  project/language detection  →  language intelligence
      ✅ done                    ✅ done                    ⏳ now
```

We deliberately support few languages well rather than many languages badly:

```
Rust → solid
Go   → solid
Python → built-in, offline
Shell  → built-in, offline
TypeScript / JavaScript → built-in, offline
C / C++ → built-in, offline (clangd when present)
Java → built-in, offline (managed Eclipse JDT)
C# → built-in, offline (managed OmniSharp)
HTML / CSS → built-in, offline (npm servers)
then expand
```

---

## Now (foundation — complete)

- [x] Rope-backed editor with cursor, selection, undo/redo, auto-indent.
- [x] Tabs, lazy file tree, git status.
- [x] Confidence-based language detection (multiple signals).
- [x] `LanguageProvider` abstraction + registry.
- [x] Syntax highlighting for Rust, Go, Python, Markdown, JSON, TOML and YAML.
- [x] Command palette, quick open, find/replace, go-to-line.
- [x] Terminal-native, transparency-friendly UI.
- [x] A strong visual identity: Mellow colours, original ASCII welcome scene,
      the Koda familiar and personality-rich empty states.
- [x] A welcome **home screen** shown on every launch: an interactive,
      keyboard-navigable menu (open file/project, create project, resume
      session, recent projects and files) around the animated familiar, with
      the command-line target preserved.
- [x] Guided **project creation**: choose a folder, name and validate it, pick a
      language, and scaffold a conventional project deterministically and
      offline, then open it ready to code.

---

## Next — make language intelligence real

The highest-value work. These turn Koda from an editor into an IDE.

- [x] **Asynchronous language work (foundation).** A background worker thread with
      a request/event channel now runs language detection and git status off the
      UI thread; the event loop applies results as they arrive. Language
      intelligence will reuse this channel.
- [x] **Diagnostics pipeline (built-in).** Providers produce diagnostics that
      flow through the background worker into the editor: gutter markers,
      underlines, a statusline count, cursor messages, navigation (`F8`/
      `Shift+F8`) and a diagnostics list. Rust and Go currently report lexical
      structural problems; richer, compiler-backed diagnostics arrive with the
      language-server backends below.
- [x] **Completion.** A compact popup merges provider keywords/types/builtins
      with identifiers from the buffer, filters as you type and accepts with
      `Enter`/`Tab`. Language-server candidates can extend the same popup.
- [x] **Automatic completion.** The popup is offered while typing, after a short
      pause, and stays quiet in comments and strings, after punctuation, and
      when there is nothing new to offer; `Ctrl+Space` remains the manual
      fallback. Stale server responses are discarded by request id.
- [x] **Hover information.** `Ctrl+Shift+H` opens a dismissible popup with the
      symbol's kind, its definition line and its usage count. Heuristic today;
      language-server hover can replace the content.
- [x] **Go-to-definition and references.** `F12` resolves the word under the
      cursor to its definition and `Shift+F12` lists every occurrence; without
      a language server, `F12` falls back to a project-wide symbol search so it
      still reaches definitions in other files.
- [x] **Document symbols and symbol navigation** (`Ctrl+Shift+O`). Rust and Go
      providers scan the active file for definitions and offer a filterable
      outline; the chosen symbol is revealed. Heuristic today, LSP symbols later.
- [x] **Project-wide symbol search.** `Ctrl+T` scans the project on the
      background worker and lists every definition in a filterable picker.
      When a language server is attached its `workspace/symbol` results are
      merged in, with the built-in scan as the instant, offline fallback.
- [x] **Formatting.** `Ctrl+Shift+I` runs the language's trusted formatter
      (`rustfmt`, `gofmt`) on a snapshot through the background worker and
      replaces the buffer as one undoable edit. Reuses system tools, detects
      whether they are installed, and can install them from **Language Setup…**.
      When a language server offers `textDocument/formatting` it is preferred,
      so Java and C# format through their server.
- [x] **Rename and code actions.**
- [x] **Tool provisioning and lifecycle.** Discovery is implemented, and
      **Language Setup…** can install a missing tool through its official
      manager (`rustup`, `go install`). When a Rust/Go file is open and its
      server is missing, Koda offers to install it once per session. Koda
      re-probes afterwards and starts a server when one becomes available.
      Discovery also searches well-known user bin directories, so a minimal
      `PATH` no longer hides an installed tool.
      Editing works offline; version pinning beyond what the package managers
      provide is still to come.
- [x] **Rust/Go intelligence backends (LSP client + diagnostics).** Koda now
      spawns `rust-analyzer`/`gopls` when installed, runs the LSP lifecycle,
      keeps documents in sync and shows server diagnostics through the existing
      diagnostics UI. It falls back to the built-in heuristics when no server is
      available. See the next section for the features still to run over LSP.
- [x] **Language-server reliability.** A handshake that never completes is
      abandoned after a timeout, a server that exits is retried automatically a
      bounded number of times, and **Restart Language Server** reconnects on
      demand. Every failure falls back to the built-in providers.
- [x] **LSP-backed completion, hover, go-to-definition and references.** When a
      server is attached these features come from it; the built-in providers
      remain as fallbacks. Rename and code actions are next.
- [x] **Rename** over LSP: `F2` prompts for a name and applies the server's
      workspace edit across files.
- [x] **Code actions** over LSP: `Ctrl+.` lists quick fixes and refactors and
      applies edits or commands, including `workspace/applyEdit`.
- [x] **One server per language.** Each language in the workspace keeps its own
      server with an independent handshake, restart budget and failure state, so
      mixed-language projects (and split panes) get tooling for every language,
      not just the first detected.
- [x] **Protocol hygiene.** Koda negotiates `utf-8` positions where the server
      offers them, only requests features the server advertises (falling back to
      its built-in providers otherwise), and reports a crashing server's stderr.
- [x] **Signature help.** Parameter hints for the call under the cursor, from
      the server's `textDocument/signatureHelp`, with the active parameter
      emphasised and overloads counted.
- [x] **Managed runtimes and Java/C# servers.** Provisioning can download and
      verify a runtime, not only run a package manager: Java installs Eclipse
      JDT with a checksum-verified Eclipse Adoptium JDK, and C# installs
      OmniSharp with the .NET SDK, both under Koda's data directory. Servers are
      launched with their runtime on `PATH` (`JAVA_HOME`, `DOTNET_ROOT`).
- [x] **LSP position encoding.** Positions are converted in both directions
      using the encoding the server negotiated (`utf-8`, `utf-16` or `utf-32`)
      and each line's text, so diagnostics, navigation and rename/format/
      code-action edits are correct on non-ASCII lines.
- [x] **Language-server response validation.** Document-sensitive requests
      record the document version at request time; stale or superseded
      responses are discarded, and requests have bounded lifetimes.

The abstraction is already in place: providers declare `Capability`s, and the
command palette already reports which are available. Filling them in is additive.

---

## Then — deepen the editing experience

- [x] Bracket matching and auto-closing pairs.
- [x] Go to matching bracket (`Alt+M`).
- [x] Delete line (`Ctrl+Shift+K`).
- [x] Selection-aware indentation, line move/duplicate, and grouped undo.
- [x] Inline fuzzy file filtering in the sidebar.
- [x] Select next occurrence (`Ctrl+D`): select the word under the cursor, then
      cycle through its whole-word occurrences.
- [ ] Multiple cursors.
- [x] Indentation width is inferred per file (two-space JavaScript, four-space
      Rust, …) with no configuration.
- [x] Indentation guides that follow each file's detected indentation width.
- [ ] A more complete tokenizer (strings, lifetimes, generics) for the Rust and
      Go providers.
- [x] Case-sensitive and whole-word search options (`Alt+C` / `Alt+W`),
      case-insensitive by default and shown in the find bar.
- [x] Regular-expression search in the find bar (`Alt+R`), powered by a small
      built-in engine.
- [x] Undo grouping for consecutive typing.
- [ ] Soft wrap.
- [x] Persist cursor position, open tabs and expanded directories per project.
- [x] A kill-ring with **Alt+Y** yank-pop; the newest kill still drives the
      system clipboard. Named registers remain to come.

---

## Then — project intelligence

- [ ] Workspace model: `Workspace → Project → Language environment → Files`.
- [x] Per-file project context: detection uses the file's **nearest** project
      markers, so a monorepo subproject is recognised even when the workspace
      root declares nothing.
- [ ] A workspace model that surfaces multiple projects and their language
      environments as one navigable structure.
- [ ] A dedicated language detection subsystem with pluggable signals and
      user-confirmation when confidence is low.
- [x] Non-code language support (Markdown, JSON, TOML, YAML) for config files.
      Each has built-in highlighting; JSON/TOML/YAML also report unbalanced
      delimiters and expose their keys (and Markdown its headings) as symbols.
- [x] Python support: built-in intelligence (highlighting, diagnostics, symbols,
      completion, hover, navigation) that works offline, plus automatic
      provisioning of the `pylsp` language server for rename, code actions and
      richer analysis.
- [x] Shell support (bash/zsh/sh): built-in highlighting, symbols, completion,
      hover and navigation, with optional `bash-language-server` provisioning.
- [x] TypeScript and JavaScript support: one built-in, offline scanner for
      highlighting (including block comments and template literals), structural
      diagnostics, symbols, completion, hover and navigation, with
      `typescript-language-server` provisioned for rename and code actions.
- [x] C and C++ support: preprocessor/comment/string highlighting, structural
      diagnostics, symbols (functions, structs, classes, enums, unions,
      typedefs, `#define`) and offline completion/hover/navigation, with
      `clangd` used when the toolchain provides it.
- [x] Java and C# built-in support: annotation/attribute highlighting, strings
      (including C# verbatim/raw strings and Java text blocks), structural
      diagnostics, symbols (types, methods, fields, records, interfaces) and
      offline completion/hover/navigation.
- [x] HTML and CSS built-in support: tag/attribute and selector/property
      highlighting, tag- and brace-balance diagnostics, id/selector symbols,
      completion and hover, with `vscode-langservers-extracted` provisioned for
      formatting and richer intelligence.
- [x] `.gitignore`-aware file tree and quick open. A dependency-free matcher
      reads root and nested `.gitignore` files plus `.git/info/exclude`, and
      hides ignored paths from the tree, quick open, the inline filter and
      workspace symbol search.
- [x] File operations: create (New File…), rename (Rename…) and delete
      (Delete…) from the tree or the active file. Renaming with a path also
      moves a file.
- [x] Copy/duplicate files (**Duplicate File**, **Copy File…**); moving works
      through **Rename…** with a path.
- [x] Save all and external-change detection: clean files reload from disk and
      dirty files are preserved with a warning.
- [x] Revert the active file to its on-disk version (**Revert File** in the
      palette).

---

## Then — polish

- [x] Animated **welcome scenes**: *starry night*, *cozy desk*, *rainy window*,
      *sakura drift* and *quiet study*, composed on a character canvas with
      clouds, a city skyline, a glowing lantern and the familiar cycling
      expressions. `v` (or **Change Welcome Scene**) cycles them.
- [x] **Toggle Animations** for a calm, reduced-motion experience; the scene and
      busy sparkle freeze when it is off.
- [x] Personable, contextual **empty states** across pickers and the file tree:
      a familiar pose, a short line and a hint instead of a bare "no matches".
- [x] Refresh the project tree and git status on terminal focus and on demand
      (`F5`).
- [x] Keyboard-shortcuts overlay (`F1`), generated from the command registry.
- [x] Overlay polish: pickers and prompts gained a result counter, a footer
      hint and a personable empty state.
- [x] Split editor: two panes side by side (`Alt+V`) with focus switching
      (`Alt+O`).
- [x] Keymap audit: fixed the terminal-aliased `Ctrl+M` rebind to `Alt+M` and
      added `Ctrl+Shift+S`, `Ctrl+N`, `F3`/`Shift+F3`, `Ctrl+PageUp/Down` and
      `Ctrl+Shift+M`.
- [x] Editor scroll margin (scrolloff) and a gentle welcome-mascot animation.
- [x] Inline diagnostic messages: an optional, severity-coloured note at the end
      of the affected line, folded into the existing diagnostics UI and
      toggleable from the palette.
- [ ] A subtle, optional theme system (works with no config by default).
- [x] A read-only unified **diff view** for git: a colour-coded, scrollable
      panel for the active file, and `d` on a changed-files entry. (An
      interactive merge view remains future work.)
- [ ] File iconography that respects monochrome terminals.
- [ ] An interactive diff/merge view for conflicting files.
- [x] A changed-files list (`Ctrl+Shift+G`) over the working-tree snapshot;
      selecting an entry opens it.
- [x] A basic commit flow: **Commit Changes…** prompts for a message, stages
      everything and commits on the background worker, then refreshes status.
- [x] A staged/unstaged git view with per-file staging: the changed-files list
      marks each path and `Space` stages or unstages it.
- [x] Project-wide text search (`Ctrl+Shift+F`): a `.gitignore`-aware,
      case-insensitive scan on the background worker with a filterable result
      list.
- [x] Notifications/toasts for long-running operations: background results
      appear as severity-coloured toasts above the statusline.

---

## Later — extensibility

- [ ] Optional user configuration (theme, keybindings, editor behaviour) with
      excellent defaults. A fresh install never requires a config file.
- [ ] A plugin/provider API for third-party languages.
- [ ] Remote development over SSH.
- [ ] Debug adapter support.

---

## Known limitations

- `.gitignore` discovery is capped (256 nested files, 4096 directories) so
  opening a huge monorepo stays predictable; a rule beyond the cap is not
  applied.
- Document synchronization is always whole-document. This is the protocol's safe
  fallback, including for servers that prefer incremental changes; incremental
  sync is not implemented.
- Recent/session state serializes paths lossily, so a non-UTF-8 path may not
  round-trip.

---

## Non-goals

- Recreating VS Code in a terminal.
- Filling the screen with panels and controls.
- Publishing a marketplace of half-working plugins.
- Requiring the user to understand LSP, install language servers, or write
  configuration before they can code.
