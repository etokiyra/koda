# Changelog

All notable changes to Koda are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

New work is collected under **Unreleased**. The released version below is Koda's
first release; it is written from the code's final state — superseded interim
entries have been collapsed rather than kept as a blow-by-blow — and is dated
when it is tagged.

## [Unreleased]

### Added

- **In-app Settings.** A keyboard-driven **Settings** screen (opened from the
  command palette) changes a small set of global preferences: theme, line
  numbers, animations, soft wrap, indentation width, spaces vs tabs, inline
  diagnostics and automatic completion. Each change applies immediately, can be
  reset to the defaults, and is remembered across launches. Koda is fully usable
  with no settings file; the choices live in
  `$XDG_STATE_HOME/koda/settings.json` (or `~/.local/state/koda/settings.json`).
  A missing file loads the defaults, and a malformed file is preserved as
  `settings.json.corrupt` rather than overwritten.
- **Three bundled themes.** **Mellow** (the default) is joined by **Midnight**,
  a high-contrast cool dark theme, and **Daylight**, a light theme. All three
  are data-driven from one semantic role set, selectable at runtime, and cover
  every existing surface (editor, statusline, tabs, file tree, palette,
  completion popup, diagnostics, overlays and diffs).
- **One-step project setup.** Opening a project runs a bounded,
  `.gitignore`-aware scan for the languages it contains, then offers a single
  **Project setup** summary. **Set up project** installs every missing managed
  language tool through the existing provisioning path; a language that is
  already ready is never reinstalled, an independent language still succeeds if
  another fails, and a missing system prerequisite is reported rather than
  attempted. **Later** (or `Esc`) dismisses it and leaves built-in editing
  working; **Set Up Project…** in the palette reopens it on demand.
- **Offline, verified provisioning cache.** A successfully verified download is
  stored by its digest under Koda's data directory and reused without
  re-downloading it, and the checksum/release metadata that names it is cached
  by URL as well, so a tool Koda has installed once can be reinstalled or
  repaired with no network. A cached artifact is re-verified before every use, a
  corrupt entry is discarded and redownloaded, and only verified artifacts are
  cached.

### Changed

- **View toggles now persist.** **Toggle Soft Wrap**, **Toggle Inline
  Diagnostics** and **Toggle Animations** update the corresponding preference,
  so the palette and the Settings screen stay in step across launches.
- **Provisioning is atomic and pre-flighted.** Downloads are written to a
  temporary file and renamed into place only after verification; extraction
  stages into a sibling directory and promotes it with a rename, restoring the
  previous installation if promotion fails, so a failed replacement no longer
  destroys a known-good tool. `install()` now refuses a plan whose steps cannot
  all run (or whose disk space is short) before it downloads anything.
- **Provisioning failures are actionable**, distinguishing no network, a proxy
  or TLS failure, a timeout and a checksum mismatch, and naming the tool and
  command that failed.
- **Hex is pinned to an exact version**, and **rebar3 is a Koda-verified
  download** registered through `mix local.rebar` instead of being fetched
  unpinned by Mix. `rustup component add` remains the one documented delegation.
- **Provisioning pre-flight and rollback boundaries** are documented in
  [docs/LIMITATIONS.md](docs/LIMITATIONS.md#provisioning).
- **The language-support contract is now explicit.** A new
  [docs/LANGUAGE-SUPPORT.md](docs/LANGUAGE-SUPPORT.md) defines what "supported"
  means (Complete / Complete-with-a-prerequisite / Offline only /
  Platform-limited), lists each language's offline capabilities, server and
  verification, runtime prerequisite and platform limits, and records the
  known capability-gating inconsistency. A test keeps it in sync with the
  provider registry.

### Security

- **The download cache never bypasses verification.** Cache identity is the
  expected digest, a cached artifact is re-hashed before use, and a failed or
  mismatched download is never written to the cache.

## [0.1.0] - 2026-10-05

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
  isolated `local::lib` for Perl's PLS. Every managed download is pinned to an
  exact version and verified against a checksum or signature and fails closed;
  extraction refuses path traversal; large installs check free disk space and
  honour `HTTP(S)_PROXY`; installs are bounded, serialised with an advisory lock,
  and re-probed by launching the real server. **Language Setup** shows a managed
  tool's version and on-disk size and can update or remove it without touching a
  user/system install.
- **Git.** Branch and per-file status, a changed-files list with per-file
  staging, a commit flow, and a read-only unified diff panel.
- **Interface.** The Mellow colourscheme, five animated welcome scenes, the Koda
  familiar, personable empty states, toasts for background work, a
  keyboard-shortcuts cheatsheet generated from the command registry, and the
  command palette.
- **Terminal.** OSC 52 clipboard, bracketed paste and focus reporting, and a
  transparent, terminal-native layout.
- **Distribution.** A tagged release publishes prebuilt archives for Linux
  x86_64 (glibc and musl) and macOS (x86_64 and arm64), each with a SHA-256
  checksum, from the GitHub Releases page. See the
  [install instructions](README.md#-install).

### Changed

- **Relicensed under the Mozilla Public License 2.0** (previously declared
  GPLv3; originally MIT). MPL-2.0 is file-level copyleft.
- Go tooling (`gopls`, `sqls`, `shfmt`, `gofmt`) now comes from Koda's managed
  official Go toolchain, and C/C++, PHP and Assembly no longer need a system
  toolchain for their servers — reducing system prerequisites across the board.
- Language setup is now offered when a project is opened, not only when a file
  is open.
- **Every managed install input is an exact version.** Eclipse JDT moved from the
  floating `-latest` snapshot to a pinned milestone with its published `.sha256`;
  `lua-language-server` and OmniSharp use a pinned tag with a GitHub digest;
  `kotlin-language-server` uses a pinned SHA-256; the .NET SDK is downloaded
  directly and verified by SHA-512 instead of running `dotnet-install.sh`; the
  Rust bootstrap downloads and verifies a pinned `rustup-init` binary instead of
  the moving `sh.rustup.rs` script; the Go toolchain and the Adoptium JDKs are
  pinned by version; and every package-manager install names an exact version.
- **Internal:** `src/app/mod.rs` (8876 lines) was split into focused modules
  under `src/app/` (mechanical extraction, no behaviour change), so every
  production app file is under 800 lines. See
  [DECISIONS.md](docs/DECISIONS.md#splitting-srcappmodrs).

### Fixed

- **LSP positions are converted correctly on non-ASCII lines** in both
  directions, using the server's negotiated encoding (UTF-8/UTF-16/UTF-32).
- **Superseded language-server responses are discarded**; a response is applied
  only when the document is still open at the same version it was requested for.
- **Tag-balance diagnostics ignore raw bodies**, so `<`/`>` inside a
  `<script>`/`<style>` block no longer produce false "never closed" warnings.
- **Checksum and extraction are portable**: hashing falls back through
  `sha256sum`/`sha512sum`, `shasum`, `openssl` and Python, and zip extraction
  falls back through `unzip`, `bsdtar` and Python.
- **Saves are atomic and lossless**, preserving permissions and refusing to
  overwrite an existing destination on rename/copy.

### Security

- **Every managed download is verified and fails closed.** A `Download` step
  cannot be constructed without a SHA-256; GitHub releases, `builds.hex.pm`,
  Adoptium, Node.js, the Dart SDK, the Go toolchain, Microsoft's .NET
  `releases.json`, `rustup-init`, the Swift signature and the pinned `cpanm`
  each verify a checksum, digest or signature. A checksum mismatch or a
  truncated download is rejected and removed before it can become an installed
  tool. Package-manager installs name an exact version and delegate integrity to
  the manager.
- Managed installs run from Koda's own data directory, so a project-local
  `.npmrc` cannot hijack `npm`; downloads and commands are time- and
  output-bounded.
- Downloaded archives are listed before extraction and refused if any entry is
  absolute or contains `..`, failing closed if the listing itself fails.
- Koda never writes to a system-owned location or modifies the user's system
  environment.

> **Known gaps at release.** Two package-manager inputs delegate to their
> manager's trust model: `rustup component add` follows the user's toolchain
> channel, and `mix local.hex`/`local.rebar` fetch Hex's signed archive. There is
> no separate estimate of Swift's unpacked size, and removing a shared managed
> component (Go tools, npm servers, the JDKs) can also remove a sibling Koda
> tool. See [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md#provisioning). Windows is
> not supported.

[Unreleased]: https://github.com/etokiyra/koda/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/etokiyra/koda/releases/tag/v0.1.0
