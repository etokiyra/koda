# Language support

The authoritative, honest statement of what Koda supports today. It is the
language-level companion to [PRODUCT.md](PRODUCT.md#what-supported-means), and it
is written from the actual implementation, not from intent. If this file and the
code disagree, the code is right and this file is a bug.

## The support contract

A language is only called **supported** when the claim matches what the code
does. Koda uses four classes:

- **Complete** — detection, built-in offline editing, and a language server Koda
  provisions itself with nothing else required from the system.
- **Complete (prerequisite)** — the same, but one system runtime or tool must
  already exist; Koda names it and does not replace it.
- **Offline only** — detection and built-in offline editing; no language server.
- **Platform-limited** — the server or toolchain is unavailable or unverified on
  some first-class platform; the limitation is stated per platform.

Anything not proven by a test or a live run is marked **unverified** rather than
assumed. "An LSP starts" is not sufficient for any class.

## What works offline

Built-in provider capabilities, verified from each provider's `capabilities()`:

- `highlight` — syntax highlighting (every language)
- `diagnostics` — structural diagnostics
- `symbols` — document symbols / outline
- `completion` — keywords, types, builtins (merged with buffer words)
- `hover` — symbol information
- `navigation` — go-to-definition and references within the file
- `format` — an external formatter Koda runs (not LSP formatting)

## User-facing matrix

<!-- languages:start -->
| Language | Works offline | Language server | Runtime needed | Server platforms | Class |
| --- | --- | --- | --- | --- | --- |
| **Rust** | highlight, diagnostics, symbols, completion, hover, navigation, format | `rust-analyzer` (rename, code actions) | none (Koda bootstraps `rustup`) | Linux glibc/musl, macOS x86_64/arm64 | Complete |
| **Go** | highlight, diagnostics, symbols, completion, hover, navigation, format | `gopls` (rename, code actions) | none (managed Go) | Linux glibc/musl, macOS x86_64/arm64 | Complete |
| **Python** | highlight, diagnostics, symbols, completion, hover, navigation | `pylsp` (rename, code actions) | `python3` | wherever `python3` runs | Complete (prerequisite) |
| **Shell** | highlight, symbols, completion, hover, navigation, format | `bash-language-server` | none (managed Node.js) | Linux, macOS | Complete |
| **TypeScript** | highlight, diagnostics, symbols, completion, hover, navigation, format | `typescript-language-server` | none (managed Node.js) | Linux, macOS | Complete |
| **JavaScript** | highlight, diagnostics, symbols, completion, hover, navigation, format | `typescript-language-server` | none (managed Node.js) | Linux, macOS | Complete |
| **C** | highlight, diagnostics, symbols, completion, hover, navigation, format | `clangd` | `clang-format` (LLVM) for formatting only | glibc Linux, macOS — **not musl** | Complete (prerequisite) |
| **C++** | highlight, diagnostics, symbols, completion, hover, navigation, format | `clangd` | `clang-format` (LLVM) for formatting only | glibc Linux, macOS — **not musl** | Complete (prerequisite) |
| **Java** | highlight, diagnostics, symbols, completion, hover, navigation | `jdtls` (rename, code actions, LSP format) | `python3` (jdtls launcher) | Linux, macOS | Complete (prerequisite) |
| **C#** | highlight, diagnostics, symbols, completion, hover, navigation | `OmniSharp` (rename, code actions, LSP format) | none (managed .NET SDK) | Linux, macOS | Complete |
| **PHP** | highlight, diagnostics, symbols, completion, hover, navigation | `phpactor` (rename, code actions) | `php` | wherever `php` runs | Complete (prerequisite) |
| **Kotlin** | highlight, diagnostics, symbols, completion, hover, navigation | `kotlin-language-server` (rename, code actions, LSP format) | none (managed JDK 21) | Linux, macOS | Complete |
| **HTML** | highlight, diagnostics, symbols, completion, hover, format | `vscode-html-language-server` (navigation, rename) | none (managed Node.js) | Linux, macOS | Complete |
| **CSS** | highlight, diagnostics, symbols, completion, hover, format | `vscode-css-language-server` (navigation, rename) | none (managed Node.js) | Linux, macOS | Complete |
| **Lua** | highlight, diagnostics, symbols, completion, hover, navigation | `lua-language-server` (rename, code actions, LSP format) | none | Linux, macOS | Complete |
| **Ruby** | highlight, diagnostics, symbols, completion, hover, navigation | `solargraph` (rename, code actions) | `ruby` + `gem` | wherever Ruby runs | Complete (prerequisite) |
| **SQL** | highlight, diagnostics, symbols, completion, hover, navigation | `sqls` (rename) | none (managed Go) | Linux glibc/musl, macOS x86_64/arm64 | Complete |
| **Assembly** | highlight, symbols, completion, hover, navigation | `asm-lsp` | none (prebuilt) or `cargo`/`rustup` | Linux x86_64 (glibc), macOS x86_64/arm64; `cargo` elsewhere | Platform-limited |
| **Perl** | highlight, diagnostics, symbols, completion, hover, navigation, format | `PLS` (fallback `Perl::LanguageServer`) | `perl` | wherever `perl` runs | Complete (prerequisite) |
| **Dart** | highlight, diagnostics, symbols, completion, hover, navigation, format | `dart language-server` | none (managed Dart SDK) | Linux, macOS | Complete |
| **Elixir** | highlight, diagnostics, symbols, completion, hover, navigation, format | `ElixirLS` | none (managed OTP + Elixir) | **Linux only** | Platform-limited |
| **Swift** | highlight, diagnostics, symbols, completion, hover, navigation, format | `sourcekit-lsp` | none (managed toolchain) | glibc Linux (native or portable UBI10); discovery elsewhere | Platform-limited |
| **Markdown** | highlight, symbols, format | — | none | all | Offline only |
| **JSON** | highlight, diagnostics, symbols, completion, format | — | none | all | Offline only |
| **TOML** | highlight, diagnostics, symbols, completion | — | none | all | Offline only |
| **YAML** | highlight, diagnostics, symbols, completion, format | — | none | all | Offline only |
<!-- languages:end -->

"Linux" without a libc qualifier means both glibc and musl unless a managed
artifact is glibc-only; those are called out explicitly.

## Server provenance (technical)

| Language | Server | Version | Verification | Runtime |
| --- | --- | --- | --- | --- |
| Rust | `rust-analyzer`, `rustfmt` | user's toolchain | delegated to `rustup` (signed manifests); `rustup-init` 1.29.1 pinned + SHA-256 when bootstrapped | — |
| Go | `gopls` | `@v0.23.0` | managed Go `go1.27.1` SHA-256; module spec pinned | managed Go |
| Python | `pylsp` | `==1.15.0` | package-manager delegation (pip/pipx/uv) | system `python3` |
| Shell | `bash-language-server` | `@5.8.1` | npm delegation; managed Node.js `24.21.0` SHA-256 | managed Node.js |
| TypeScript / JavaScript | `typescript-language-server` + `typescript` | `@6.0.1` + `@7.0.2` | npm delegation; managed Node.js | managed Node.js |
| C / C++ | `clangd` | `23.1.0` | GitHub asset SHA-256; glibc-only | — |
| C / C++ format | `clang-format` | user's LLVM | discovered, not managed | system LLVM |
| Java | `jdtls` + Adoptium JDK 25 | `1.61.0` milestone + `jdk-25.0.4.1+1` | published `.sha256`; Adoptium API checksum | managed JDK |
| C# | `OmniSharp` + .NET SDK | `v2.0.0` + `10.0.401` | GitHub asset SHA-256; `releases.json` SHA-512 | managed .NET |
| PHP | `phpactor` | `2026.06.23.0` | GitHub asset SHA-256 | system `php` |
| Kotlin | `kotlin-language-server` + Adoptium JDK 21 | `1.3.13` + `jdk-21.0.12.1+1` | Koda-pinned SHA-256; Adoptium API checksum | managed JDK 21 |
| HTML / CSS | `vscode-langservers-extracted` | `@4.10.0` | npm delegation; managed Node.js | managed Node.js |
| Lua | `lua-language-server` | `3.19.1` | GitHub asset SHA-256 | — |
| Ruby | `solargraph` | `0.60.4` | RubyGems delegation into an isolated gem home | system `ruby`/`gem` |
| SQL | `sqls` | `@v0.2.48` | module spec pinned; managed Go | managed Go |
| Assembly | `asm-lsp` | `0.10.1` | GitHub asset SHA-256; `cargo install --version` fallback | — or `cargo` |
| Perl | `PLS` (fallback `Perl::LanguageServer` `2.6.2`) | `@0.906` | checksum-verified `cpanm` `1.7049`; isolated `local::lib` | system `perl` |
| Dart | Dart SDK | `3.13.5` | sibling `.sha256sum` | managed SDK |
| Elixir | `ElixirLS` + OTP + Elixir | `0.31.1` + `27.3.4` + `1.18.4` | GitHub asset SHA-256; `builds.hex.pm` checksums; Hex `2.5.1` pinned; rebar3 `3.24.0` Koda-verified | managed runtime |
| Swift | `sourcekit-lsp` | `6.4.0` | detached GPG signature | managed toolchain |

Full provisioning semantics (cache, atomicity, pre-flight, offline reuse) are in
[DECISIONS.md](DECISIONS.md#provisioning) and
[LIMITATIONS.md](LIMITATIONS.md#provisioning).

## Verification status

Live server handshakes (and, where noted, completion/diagnostics round-trips) were
run on **Linux glibc x86_64** for: `gopls`, `clangd`, `asm-lsp`, `PLS`, `sqls`,
`dart language-server`, `ElixirLS`, `lua-language-server`,
`kotlin-language-server`, `vscode-html-language-server` and
`vscode-css-language-server`.

**Not live-verified** anywhere in this repository's audits: `rust-analyzer`,
`pylsp`, `bash-language-server`, `typescript-language-server`, `jdtls`,
`OmniSharp`, `phpactor`, `solargraph`, `sourcekit-lsp` (macOS), and every server
on **Linux musl** and **macOS**. They are build/unit-verified and, for some, were
installed successfully, but a handshake was not run. Treat their runtime status
as unverified until a live test covers it.

## Platform notes

- **Linux musl.** `clangd` and the prebuilt `asm-lsp` are glibc-only and are
  withheld; C/C++ falls back to a system `clangd`, Assembly to `cargo install`.
  The Go toolchain, Node.js, Dart, JVM, .NET and Lua artifacts are not
  glibc-only.
- **macOS.** The managed artifacts that exist cover x86_64 and arm64. Elixir is
  Linux-only (`bob` publishes no macOS build). Swift is discovery-only off
  swift.org's Linux distributions.
- **Other architectures.** Only where upstream publishes an artifact; otherwise
  Koda reuses the user's tool. Linux aarch64 has no prebuilt `asm-lsp`.

## Known inconsistency: the capability gate

`Capability` is used for two different jobs, and they disagree:

- Providers declare **built-in** capabilities, and `command_availability` gates
  the palette on that list for `Format Document`, `Rename Symbol` and
  `Code Actions`.
- Those three are actually **LSP-backed** and work for any attached server that
  advertises the request (the keybindings do not consult the capability list).

Consequently `Rename`/`Code actions` are only *advertised* for Rust and Go, and
`Format Document` is only advertised when the provider declares built-in
formatting — even though `jdtls`, `OmniSharp`, `pylsp` and others provide them
over LSP. The features work; the palette under-reports them. This is recorded
here rather than fixed in this phase; the fix is to consult the attached server's
advertised capabilities for server-backed commands and keep provider
capabilities for built-in features.

## PHP and Ruby

Both have solid built-in offline editing and a managed server package
(`phpactor.phar`, `solargraph`), but **Koda does not provision their runtime**:

- Official PHP is source-only; the third-party static builds publish no
  checksum, so Koda cannot verify one.
- Ruby has no official portable binary.

Under the zero-configuration principle, their servers are therefore **Complete
(prerequisite)**, not Complete: the user must have `php` or `ruby`/`gem` on
`PATH`. Editing, highlighting, diagnostics, symbols, completion, hover and
navigation work offline with no runtime at all. Provisioning a runtime would be
a product decision, not a documentation one; see
[LIMITATIONS.md](LIMITATIONS.md#provisioning).

## Adding a language

Adding a language should remain cheap: a `LanguageProvider` with a detection
descriptor and built-in features, registered in `ProviderRegistry::builtin()`,
plus (when it has a server) an entry in the `Tool` registry. The provider half is
already uniform; the `Tool` half is still a `Tool` enum with per-tool match arms
(`language()`, `serves()`, `install_hint()`, `install_attempts()`), so the same
language is named in several places. Making that descriptor-driven is the
roadmap item, not a decision taken here.
