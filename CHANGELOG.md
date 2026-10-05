# Changelog

All notable changes to Koda are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

The version below is the proposed first release. It is written from the code's
final state — superseded interim entries have been collapsed rather than kept as
a blow-by-blow — and it carries no date until it is actually tagged.

## [Unreleased]

_Nothing yet._

## [0.1.0] - unreleased

The first release: a zero-configuration terminal IDE that opens a project and
starts working.

### Added

- **Editor.** Rope-backed buffers with multi-document tabs, cursor and
  selection, operation-based undo/redo with a save point, auto-indent and
  bracket auto-pairing, matching-bracket highlighting, grouped multi-cursor
  editing (`Ctrl+D`/`Ctrl+Shift+L`, stacked carets), a kill-ring with `Alt+Y`
  yank-pop, selection-aware indent/outdent, line move/duplicate/delete, and a
  split editor (`Alt+V`, `Alt+O`).
- **Soft wrap.** Display-only wrapping (`Alt+Z`) with visual-row Up/Down
  movement and no effect on positions, undo, diagnostics or cursors.
- **Files and projects.** A welcome home screen with recent projects and files
  and session resume; a lazy, `.gitignore`-aware file tree with an inline fuzzy
  filter; quick open; file create/rename/delete/duplicate/copy; save-all,
  revert, and external-change detection; deterministic, offline project
  scaffolding; and per-project session persistence (open tabs, cursors,
  expanded folders).
- **Search.** Find/replace with case, whole-word and regex options; project-wide
  text search; document, workspace and project-wide symbol navigation.
- **Language intelligence.** Confidence-based, file-first language detection and
  a provider registry. Offline built-in highlighting, structural diagnostics,
  symbols, completion, hover and navigation for **Rust, Go, Python, Shell,
  TypeScript, JavaScript, C, C++, Java, C#, PHP, Kotlin, Lua, SQL, Ruby, Perl,
  Assembly, Dart, Elixir, Swift, HTML, CSS, Markdown, JSON, TOML and YAML**.
  Python, Shell, TypeScript/JavaScript, C/C++, Java, C#, PHP, Kotlin, Lua, Ruby,
  SQL, Perl, Dart, Elixir, Swift, HTML and CSS gain richer features when a
  language server is attached.
- **Language servers.** A JSON-RPC client with lifecycle management: diagnostics,
  completion (automatic and manual), hover, signature help, go-to-definition,
  find-references, document and workspace symbols, rename, code actions and
  formatting, with the built-in providers as offline fallbacks; one server per
  language with an independent restart budget.
- **Provisioning.** **Language Setup…** discovers and installs missing tools.
  Managed, user-local installs for the official Go toolchain (which provides
  `gopls`, `sqls` and `shfmt`), the self-contained clangd bundle, `phpactor.phar`,
  the Dart SDK, a managed Node.js runtime (for the npm-based servers), an
  Eclipse Adoptium JDK (jdtls), a dedicated JDK 21 (Kotlin), a coordinated
  Erlang/OTP + Elixir + ElixirLS stack, the Swift toolchain, and `cpanm` into an
  isolated `local::lib` for Perl's PLS. Downloads verify a published checksum or
  signature where one exists and fail closed; extraction refuses path traversal;
  installs are bounded, serialised with an advisory lock, and re-probed by
  launching the real server.
- **Git.** Branch and per-file status, a changed-files list with per-file
  staging, a commit flow, and a read-only unified diff panel.
- **Interface.** The Mellow colourscheme, five animated welcome scenes, the Koda
  familiar, personable empty states, toasts for background work, a
  keyboard-shortcuts cheatsheet generated from the command registry, and the
  command palette.
- **Terminal.** OSC 52 clipboard, bracketed paste and focus reporting, and a
  transparent, terminal-native layout.

### Changed

- **Relicensed under the Mozilla Public License 2.0** (previously declared
  GPLv3; originally MIT). MPL-2.0 is file-level copyleft.
- Go tooling (`gopls`, `sqls`, `shfmt`, `gofmt`) now comes from Koda's managed
  official Go toolchain, and C/C++, PHP and Assembly no longer need a system
  toolchain for their servers — reducing system prerequisites across the board.
- Language setup is now offered when a project is opened, not only when a file
  is open.

### Fixed

- **LSP positions are converted correctly on non-ASCII lines** in both
  directions, using the server's negotiated encoding (UTF-8/UTF-16/UTF-32).
- **Superseded language-server responses are discarded**; a response is applied
  only when the document is still open at the same version it was requested for.
- **Tag-balance diagnostics ignore raw bodies**, so `<`/`>` inside a
  `<script>`/`<style>` block no longer produce false "never closed" warnings.
- **Checksum and extraction are portable**: hashing falls back through
  `sha256sum`, `shasum`, `openssl` and Python, and zip extraction falls back
  through `unzip`, `bsdtar` and Python.
- **Saves are atomic and lossless**, preserving permissions and refusing to
  overwrite an existing destination on rename/copy.

### Security

- Managed installs run from Koda's own data directory, so a project-local
  `.npmrc` cannot hijack `npm`; downloads and commands are time- and
  output-bounded, and checksum verification fails closed.
- Downloaded archives are listed before extraction and refused if any entry is
  absolute or contains `..`, failing closed if the listing itself fails.
- Koda never writes to a system-owned location or modifies the user's system
  environment.

> **Known gaps at release.** Four managed downloads are not yet checksum- or
> signature-verified — Eclipse JDT (the floating `jdtls` `-latest` snapshot),
> `lua-language-server`, `kotlin-language-server` and OmniSharp (with
> `dotnet-install.sh`). Package-manager installs delegate integrity to that
> manager. See [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md#provisioning). Windows
> is not supported.

[Unreleased]: https://github.com/etokiyra/koda/compare/master...HEAD
[0.1.0]: https://github.com/etokiyra/koda/releases/tag/v0.1.0
