# Architecture

The single source of truth for Koda's module layout and dependency rules.
`AGENTS.md` and `README.md` link here instead of keeping their own copies.

---

## Source tree

```text
src/
├── main.rs                 # CLI entry point (thin)
├── lib.rs                  # module tree
├── app/
│   ├── mod.rs              # App state, event loop, command dispatch
│   └── overlay.rs          # picker/prompt/search state + fuzzy matching
├── background.rs           # worker pool for detection, LSP, git, formatting
├── commands/mod.rs         # command registry (ids, titles, shortcuts)
├── editor/
│   ├── buffer.rs           # rope-backed text buffer
│   ├── document.rs         # buffer + cursor + selection + undo + highlight cache
│   ├── history.rs          # undo/redo edits
│   ├── layout.rs           # tab expansion + soft-wrap segments
│   ├── position.rs         # Position / Selection / Cursor
│   └── mod.rs              # Editor (open documents / tabs)
├── filesystem/
│   ├── mod.rs              # fs helpers (sorted reads, atomic writes, walking)
│   └── gitignore.rs        # .gitignore matcher
├── git/mod.rs              # branch + file status via the `git` binary
├── language/
│   ├── id.rs               # LanguageId
│   ├── mod.rs              # LanguageService facade + Capability
│   ├── symbols.rs          # workspace symbol scan
│   ├── format.rs           # formatter pipeline (stdin/stdout tools)
│   ├── tools.rs            # discovery + trusted provisioning
│   ├── detection/          # engine, signals, confidence
│   ├── provider/           # LanguageProvider trait + ProviderRegistry
│   ├── lsp/                # JSON-RPC client, lifecycle, result conversion
│   │   ├── mod.rs
│   │   ├── jsonrpc.rs
│   │   └── convert.rs      # the char ↔ LSP position boundary
│   └── <name>/mod.rs       # one provider per language (rust, go, …)
├── process.rs              # wait_captured: bounded, pipe-draining subprocess
├── project/
│   ├── mod.rs              # Project detection + Workspace
│   ├── create.rs           # deterministic, offline project templates
│   └── file_tree.rs        # lazily loaded tree
├── recent.rs               # recent projects and files for the welcome screen
├── regex.rs                # a small regex engine for search
├── search.rs               # project-wide text search
├── session.rs              # per-project session persistence
├── terminal/mod.rs         # init/restore, OSC 52 clipboard
└── ui/
    ├── theme.rs            # Mellow palette + semantic roles (single source of truth)
    ├── art.rs              # original ASCII art: mascot, wordmark, frames
    ├── header.rs           # identity bar / breadcrumb
    ├── tabs.rs             # tab pills
    ├── editor.rs           # gutter, syntax, cursorline, indent guides
    ├── file_tree.rs        # project sidebar
    ├── status_bar.rs       # the Mellow statusline
    ├── overlay.rs          # palette, quick open, prompts, directory picker
    ├── welcome.rs          # the welcome home screen
    └── mod.rs              # layout + entry point
```

Language providers: `asm`, `c`, `csharp`, `css`, `dart`, `elixir`, `go`,
`html`, `java`, `json`, `kotlin`, `lua`, `markdown`, `perl`, `php`, `python`,
`ruby`, `rust`, `shell`, `sql`, `swift`, `toml`, `web` (TypeScript/JavaScript),
`yaml`.

---

## Dependency direction

- `editor`, `filesystem`, `git`, `commands` **must not** depend on `ui` or `app`.
- `language` depends on nothing UI-related.
- `ui` may read `app` state and editor/language data, but must not contain
  editing logic.
- `app` is the conductor and may use everything.

The editor core must remain language-agnostic.

---

## Adding a language

To add a language:

1. Create `language/<name>/mod.rs` implementing `LanguageProvider`.
2. Register it in `ProviderRegistry::builtin()`.
3. That's it — detection and highlighting work automatically.

Adding a language must never require touching `editor` or `ui`.

---

## Language detection vs. language support

These are separate concerns and must stay separate:

- **Detection** (`language/detection`) consumes lightweight
  `LanguageDescriptor`s and combines signals into a `DetectionResult`
  (`language`, `confidence`, `reasons`). It never imports provider
  implementations.
- **Providers** (`language/provider` + each `language/<name>`) implement the
  `LanguageProvider` trait and *produce* those descriptors via `descriptor()`.

### Detection signals (scored)

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
