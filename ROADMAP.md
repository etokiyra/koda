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
then expand
```

---

## Now (foundation — complete)

- [x] Rope-backed editor with cursor, selection, undo/redo, auto-indent.
- [x] Tabs, lazy file tree, git status.
- [x] Confidence-based language detection (multiple signals).
- [x] `LanguageProvider` abstraction + registry.
- [x] Rust and Go syntax highlighting.
- [x] Command palette, quick open, find/replace, go-to-line.
- [x] Terminal-native, transparency-friendly UI.
- [x] A strong visual identity: Mellow colours, original ASCII welcome scene,
      the Koda familiar and personality-rich empty states.

---

## Next — make language intelligence real

The highest-value work. These turn Koda from an editor into an IDE.

- [x] **Asynchronous language work (foundation).** A background worker thread with
      a request/event channel now runs language detection and git status off the
      UI thread; the event loop applies results as they arrive. Language
      intelligence will reuse this channel.
- [ ] **Diagnostics pipeline.** Provider → diagnostics store → inline markers and
      a diagnostics list.
- [ ] **Completion.** A completion popup driven by providers, with filtering and
      acceptance via `Tab`/`Enter`.
- [ ] **Hover information.** Show type/docs for the symbol under the cursor.
- [ ] **Go-to-definition and references.** Jump within and across files.
- [ ] **Document symbols and symbol navigation** (`Ctrl+Shift+O`).
- [ ] **Formatting.** Provider-driven document/selection formatting.
- [ ] **Rename and code actions.**
- [ ] **Rust/Go intelligence backends.** Integrate mature language tooling
      internally (e.g. `rust-analyzer`, `gopls`), auto-discovered and launched by
      Koda. The user must never install or configure an LSP.

The abstraction is already in place: providers declare `Capability`s, and the
command palette already reports which are available. Filling them in is additive.

---

## Then — deepen the editing experience

- [x] Bracket matching and auto-closing pairs.
- [x] Selection-aware indentation, line move/duplicate, and grouped undo.
- [x] Inline fuzzy file filtering in the sidebar.
- [ ] Multiple cursors.
- [ ] Indentation guides and a more complete tokenizer (strings, lifetimes,
      generics) for both providers.
- [ ] Incremental search options (case sensitivity, whole word, regex).
- [ ] Undo grouping for consecutive typing.
- [ ] Soft wrap and a configurable tab width.
- [ ] Persist cursor position, open tabs and expanded directories per project.
- [ ] A kill-ring/registers model for copy/paste.

---

## Then — project intelligence

- [ ] Workspace model: `Workspace → Project → Language environment → Files`.
- [ ] Detect multiple projects/languages inside one workspace.
- [ ] A dedicated language detection subsystem with pluggable signals and
      user-confirmation when confidence is low.
- [ ] Non-code language support (Markdown, JSON, TOML, YAML) for config files.
- [ ] `.gitignore`-aware file tree and quick open.
- [ ] File operations: create, rename, delete, move.
- [ ] Save all, revert, and external-change detection.

---

## Then — polish

- [ ] Split editor.
- [ ] A subtle, optional theme system (works with no config by default).
- [ ] File iconography that respects monochrome terminals.
- [ ] Better diff/merge view for git.
- [ ] Staged/unstaged git view and basic commit flow.
- [ ] Incremental search across the project.
- [ ] Notifications/toasts for long-running operations.

---

## Later — extensibility

- [ ] Optional user configuration (theme, keybindings, editor behaviour) with
      excellent defaults. A fresh install never requires a config file.
- [ ] A plugin/provider API for third-party languages.
- [ ] Remote development over SSH.
- [ ] Debug adapter support.

---

## Non-goals

- Recreating VS Code in a terminal.
- Filling the screen with panels and controls.
- Publishing a marketplace of half-working plugins.
- Requiring the user to understand LSP, install language servers, or write
  configuration before they can code.
