# Koda

<p align="center"><em>A modern, lightweight, terminal-native IDE.</em></p>

<p align="center">
  <a href="LICENSE"><img alt="License: MPL 2.0" src="https://img.shields.io/badge/license-MPL--2.0-90b99f?style=flat-square"></a>
  <img alt="Rust 2024" src="https://img.shields.io/badge/rust-2024-ea83a5?style=flat-square&logo=rust&logoColor=white">
  <a href="https://github.com/etokiyra/koda/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/etokiyra/koda/actions/workflows/ci.yml/badge.svg?branch=master"></a>
  <a href="https://github.com/etokiyra/koda/commits/master"><img alt="Last commit" src="https://img.shields.io/github/last-commit/etokiyra/koda?style=flat-square&color=e29eca"></a>
  <a href="https://github.com/etokiyra/koda/stargazers"><img alt="Stars" src="https://img.shields.io/github/stars/etokiyra/koda?style=flat-square&color=aca1cf"></a>
  <a href="https://github.com/etokiyra/koda/issues"><img alt="Issues" src="https://img.shields.io/github/issues/etokiyra/koda?style=flat-square&color=e6b99d"></a>
</p>

<p align="center">
  <img alt="Zero configuration" src="https://img.shields.io/badge/config-zero-ea83a5?style=flat-square">
  <img alt="Terminal native" src="https://img.shields.io/badge/terminal-native-aca1cf?style=flat-square">
  <a href="https://github.com/helix-editor/helix/blob/master/runtime/themes/mellow.toml"><img alt="Theme: Mellow" src="https://img.shields.io/badge/theme-Mellow-e29eca?style=flat-square"></a>
  <img alt="Made with Rust" src="https://img.shields.io/badge/made%20with-Rust-b9aeda?style=flat-square&logo=rust&logoColor=white">
  <a href="https://github.com/etokiyra/koda/issues"><img alt="PRs welcome" src="https://img.shields.io/badge/PRs-welcome-90b99f?style=flat-square"></a>
</p>

```text
    ✦ ───────────────────────────────────────── ☾

     _  _____  ____    _
    | |/ / _ \|  _ \  / \
    | ' / | | | | | |/ _ \
    | . \ |_| | |_| / ___ \
    |_|\_\___/|____/_/   \_\

    ✦   your cozy little coding space   ✦

                   /\___/\
                   ( ･ω･ )
                    > ω <
                   /|   |\

    ✦ ───────────────────────────────────────── ☾
```

Koda is not another terminal text editor. It is a small development environment
that happens to live in your terminal — fast, minimal, keyboard-first and,
above all, **zero configuration**.

> Install Koda. Open a project. Start coding.

No LSP config. No plugin marketplace. No dotfiles to hand-write. Koda detects the
language, brings its own intelligence, and lets you get on with the interesting
part.

```text
 ✦ koda  ·  Rust  ·  src/main.rs                                    ✧  ·  ✧
 ✦ files ────────────────────────────│ ▏ main.rs
 ▏ ▸ src                             │ ● 1  use std::io;
   ▸ tests                           │   2
   · Cargo.toml                      │   3  fn main() -> io::Result<()> {
   · CHANGELOG.md                    │   4      let name = "koda";
   · README.md                       │   5      println!("hello from {name}");
   · ROADMAP.md                      │   6      Ok(())
                                     │   7  }
 Rust  1✖  ☾ main 2±  ✧ lsp           LF   Ln 5, Col 16   ☾ 42%
```

---

## ✦ Why Koda

- **Zero configuration.** Supported languages work out of the box. Koda writes
  no config, asks for none, and needs none.
- **Koda owns language intelligence.** Detection, providers and the language
  server lifecycle are first-class subsystems — including fetching the tools
  you are missing, from trusted package managers, with one keystroke.
- **Terminal-native.** Koda cooperates with your terminal. It never paints an
  opaque background, so transparency, blur and wallpaper keep working.
- **Keyboard-first.** Familiar shortcuts for muscle memory, plus a command
  palette that makes everything discoverable.
- **Fast by design.** A rope-backed editor, a lazy file tree, background
  workers, and a UI that only repaints when something actually changes.

## ☾ Language intelligence

Koda layers real IDE features on top of a clean provider abstraction. When a
language server is available it takes over; when it is not, built-in providers
keep you productive **offline**.

- **Diagnostics** — gutter markers, underlines, a statusline count, `F8`/`Shift+F8`
  navigation, a diagnostics list, and an optional inline note at the end of the
  affected line (**Toggle Inline Diagnostics**, `Alt+I`).
- **Completion** — appears automatically as you type and merges buffer
  identifiers, language keywords, and server candidates. Matching is fuzzy and
  prefix-biased, so `mrs` finds `main_result` while exact prefixes still rank
  first. It stays quiet in comments and strings; `Ctrl+Space` opens it manually
  as a fallback.
- **Hover** — `Ctrl+Shift+H`, with the symbol's kind, definition and usage count.
- **Signature help** — parameter hints appear automatically as you type `(` or `,`
  in a call, highlighting the argument you are on. It comes from the language
  server and disappears when you leave the call.
- **Navigation** — `F12` go-to-definition and `Shift+F12` find-references,
  across files when a server is attached; without one, `F12` falls back to a
  project-wide symbol search so it still works offline.
- **Symbols** — `Ctrl+Shift+O` for the file, `Ctrl+T` for the whole workspace
  (from the language server when attached, with Koda's built-in scan as the
  instant, offline fallback).
- **Rename** — `F2`, applying a workspace edit across every affected file.
- **Code actions** — `Ctrl+.` for quick fixes and refactors.
- **Formatting** — `Ctrl+Shift+I` through the language server when it offers one
  (Java, C#, TypeScript/JavaScript, …) or the language's own tool (`rustfmt`,
  `gofmt`), never blocking the UI.

### Zero-configuration language support

The complexity lives inside Koda:

```text
   open a file ──▶ detect language ──▶ choose a provider
                        │
                        ├─▶ highlight · diagnostics · symbols · completion · hover
                        │
                        └─▶ if a language server is installed
                              start it · sync documents · route features through it
                              (and it stays out of your way when none exists)
```

Missing something? Opening a project detects the languages it contains and
offers a single **Project setup** action that installs every missing language
tool; Koda also notices when a supported file is open without its server and
offers to install it, once, without blocking startup. Most
servers are provisioned from a trusted, verified source: the **official Go
toolchain** (which in turn provides `gopls`, `sqls` and `shfmt`), a
**self-contained clangd bundle**, the **.NET SDK** for C#, a **checksum-verified
Eclipse Adoptium JDK** for Java, an isolated **JDK 21** for Kotlin, a **Dart
SDK**, a **GPG-verified Swift toolchain** (native or the portable build with a
compatibility layer), a coordinated **Erlang/OTP + Elixir + ElixirLS** stack,
**PLS** (or `Perl::LanguageServer`) in an isolated `local::lib`, the prebuilt
**asm-lsp** release, the **phpactor.phar** release, and **solargraph** into a
Koda-private gem home. A system `clangd`, `php`, `ruby`, `perl` or `go` is reused
when present, and each prerequisite that is genuinely required is reported
plainly. Everything lives under Koda's own data directory.

Every managed component is checksum- or signature-verified before use, and every
install input is an exact version. The one remaining package-manager delegation
is `rustup component add`, which follows the user's toolchain. See
[`SECURITY.md`](SECURITY.md) for the full trust model.

**Set Up Project…** in the command palette (or the offer after opening a
project) plans and installs a whole project's missing tooling in one step;
**Language Setup…** lists every tool Koda knows about and installs a missing one
with a single `Enter`. The per-language table below names
each server and how it is obtained. Formatting runs the language's trusted tool —
`rustfmt`, `gofmt`, `prettier`, `clang-format`, `shfmt`, `perltidy`, or the
managed `dart format` and `mix format` — on a buffer snapshot, so unsaved edits
format in place.

### Managed toolchains

Where an official, verifiable distribution exists, Koda installs and configures
the toolchain itself. Each managed component is isolated under Koda's data
directory, launched with only the environment it needs, and verified after
installation by running the real tool — never just because a file exists.

| Component | Source | Integrity | Platforms |
| --- | --- | --- | --- |
| Swift toolchain | `download.swift.org` | GPG signature (`all-keys.asc`), fail-closed | native builds for Ubuntu 22.04/24.04/26.04, Debian 12/13, Fedora 39/41, Amazon Linux 2/2023; the portable UBI10 build plus a compatibility layer on other glibc Linux |
| Erlang/OTP + Elixir | `builds.hex.pm` (Erlang Ecosystem Foundation) | SHA-256 from `builds.txt`, fail-closed | glibc Linux (best-effort Ubuntu 24.04 target where unnamed; musl excluded) |
| ElixirLS | official GitHub release | SHA-256 asset digest | same as Erlang/Elixir |
| Go toolchain | `go.dev/dl` | SHA-256 from the release JSON, fail-closed | Linux/macOS, x86_64/arm64 |
| `rustup-init` | `static.rust-lang.org` | SHA-256 beside the versioned binary | Linux/macOS |
| clangd | `clangd/clangd` release | SHA-256 asset digest | Linux/macOS (glibc); not musl |
| asm-lsp | `bergercookie/asm-lsp` release (prebuilt) | SHA-256 asset digest | Linux x86_64, macOS x86_64/arm64; `cargo`/`rustup` elsewhere |
| phpactor | `phpactor/phpactor` release (`phpactor.phar`) | SHA-256 asset digest | wherever a system `php` exists |
| Dart SDK | Google `dart-archive` | sibling `.sha256sum`, fail-closed | Linux/macOS, x86_64/arm64 |
| Node.js | `nodejs.org` | `SHASUMS256.txt` | Linux/macOS, x86_64/arm64 |
| Eclipse Adoptium JDK | `api.adoptium.net` | checksum from the API, for a pinned release | Linux/macOS, x86_64/arm64 |
| Java (jdtls) | Eclipse milestone + managed JDK 25 | SHA-256 from the published `.sha256` | Linux/macOS, with a managed JDK |
| Kotlin | pinned server + managed JDK 21 | SHA-256 pinned in Koda's source | Linux/macOS, with a managed JDK |
| Lua | `LuaLS` release | SHA-256 asset digest | Linux/macOS |
| OmniSharp | GitHub release | SHA-256 asset digest | Linux/macOS |
| .NET SDK | `builds.dotnet.microsoft.com` | SHA-512 from `releases.json` | Linux/macOS |
| Perl (PLS, and `Perl::LanguageServer`) | CPAN, via a checksum-verified `cpanm` | SHA-256 (`App::cpanminus`), fail-closed | wherever a system `perl` exists |
| solargraph | RubyGems, into a Koda-private gem home | RubyGems signatures/HTTPS | wherever a system `gem` exists |

Every managed download is checksum- or signature-verified. A few package-manager
installs (`rustup component add`, `mix local.hex`/`local.rebar`) are pinned only
to the package manager's own trust model; see
[`docs/LIMITATIONS.md`](docs/LIMITATIONS.md#provisioning).

On a glibc distribution swift.org does not build for, Koda uses its **UBI10**
toolchain and a **managed compatibility layer**: a `libncurses.so.6` alias to the
system's `libncursesw.so.6` (the same library) and a link to the system's
`libxml2.so.2`. Nothing in the system is copied or modified — the links live
under Koda's data directory and are used through a scoped `LD_LIBRARY_PATH`. If
the distribution's `libxml2.so.2` is missing, Koda names the package to install
(on Arch: `libxml2-legacy`) and keeps built-in editing until it is present.

A large managed download (the Swift toolchain, the Dart SDK) is described with an
approximate size before it starts, and a platform Koda cannot build for is
reported as such rather than attempted.

Servers start lazily, recover automatically if they exit, and fall back to the
built-in providers whenever one is unavailable, so editing never depends on
them. Koda asks each server for `utf-8` character offsets when it offers them,
so positions stay accurate in non-ASCII files, and only uses features the server
advertises — built-in intelligence fills any gap.

Every install targets a directory you can write to — `rustup` under
`~/.cargo`, `go install` under `~/go`, a Python virtualenv, npm prefix, .NET SDK
and JDK that Koda manages under its own data directory, and
`pip --user`/`pipx` under `~/.local` — so Koda never needs `sudo` and a
system-owned prefix can never make provisioning fail. The Python virtualenv
seeds its own `pip`, so a Python without the `pip` module (or one that is
externally managed) still works. Managed servers are launched with those
runtimes on their `PATH` and a private, scoped environment (`DOTNET_ROOT`,
`JAVA_HOME`, `PERL5LIB`, `MIX_HOME`/`HEX_HOME`), so the user's system
environment is left untouched. Installation is serialised with an advisory lock
(with stale-lock recovery), so two Koda instances cannot corrupt the same
managed prefix.

### Supported languages

One table. Every language has built-in, offline intelligence; a language server
is optional and, when present, takes over the feature (`+ LSP`) with the
built-in provider as the fallback. The **authoritative per-language contract** —
offline capabilities, runtime prerequisites, platform limits and verification
status — is [docs/LANGUAGE-SUPPORT.md](docs/LANGUAGE-SUPPORT.md).

| Language | Syntax | Format | Diagnostics | Symbols | Completion | Hover | Navigation | Rename | Language server | Koda's setup |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Rust | built-in | `rustfmt` | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `rust-analyzer` | `rustup`, bootstrapped if missing |
| Go | built-in | `gofmt` | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `gopls` | `go install` via managed Go |
| Python | built-in | — | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `pylsp` | managed virtualenv (needs `python3`) |
| TypeScript | built-in | `prettier` | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `typescript-language-server` | managed Node.js |
| JavaScript | built-in | `prettier` | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `typescript-language-server` | managed Node.js |
| C | built-in | `clang-format` | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `clangd` | self-contained bundle (glibc/macOS); `clang-format` needs LLVM |
| C++ | built-in | `clang-format` | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `clangd` | self-contained bundle (glibc/macOS); `clang-format` needs LLVM |
| Java | built-in | LSP | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `jdtls` | managed JDK 25 + Eclipse JDT (needs `python3`) |
| C# | built-in | LSP | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `OmniSharp` | managed .NET SDK |
| PHP | built-in | — | built-in | built-in | built-in | built-in | built-in | LSP | `phpactor` | `phpactor.phar` (needs PHP) |
| Kotlin | built-in | LSP | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `kotlin-language-server` | managed JDK 21 + pinned server |
| HTML | built-in | `prettier` | built-in + LSP | built-in (ids) | built-in | built-in | LSP | LSP | `vscode-html-language-server` | managed Node.js |
| CSS | built-in | `prettier` | built-in + LSP | built-in (selectors) | built-in | built-in | LSP | LSP | `vscode-css-language-server` | managed Node.js |
| Lua | built-in | LSP | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP | `lua-language-server` | self-contained archive |
| Ruby | built-in | — | built-in | built-in | built-in | built-in | built-in | LSP | `solargraph` | isolated gem home (needs Ruby) |
| SQL | built-in | — | built-in | built-in | built-in | built-in | built-in | LSP | `sqls` | `go install` via managed Go |
| Perl | built-in | `perltidy` | built-in | built-in | built-in | built-in | built-in | LSP | `PLS` (fallback `Perl::LanguageServer`) | checksum-verified `cpanm` + `local::lib` (needs Perl) |
| Dart | built-in | `dart format` | built-in | built-in | built-in | built-in | built-in | LSP | `dart language-server` | managed Dart SDK |
| Elixir | built-in | `mix format` | built-in | built-in | built-in | built-in | built-in | LSP | `ElixirLS` | managed Erlang/OTP + Elixir + ElixirLS (Linux only) |
| Swift | built-in | — | built-in | built-in | built-in | built-in | built-in | LSP | `sourcekit-lsp` | GPG-verified toolchain (platform-limited) |
| Assembly | built-in | — | — | built-in (labels) | built-in | built-in | built-in | — | `asm-lsp` | prebuilt release; `cargo` fallback |
| Shell | built-in | `shfmt` | LSP | built-in (functions) | built-in | built-in | built-in | — | `bash-language-server` | `npm`, managed prefix |
| Markdown | built-in | `prettier` | — | built-in (headings) | — | — | — | — | — | offline only |
| JSON | built-in | `prettier` | built-in | built-in (top-level keys) | built-in (literals) | — | — | — | — | offline only |
| TOML | built-in | — | built-in | built-in (tables & keys) | built-in (literals) | — | — | — | — | offline only |
| YAML | built-in | `prettier` | built-in | built-in (top-level keys) | built-in (literals) | — | — | — | — | offline only |

`LSP` in the Format column means the attached server formats the file when it
offers formatting. `Rename` and code actions are LSP-backed too; they work for
any attached server that supports them, although the command palette currently
only *advertises* them where the built-in provider declares the capability (see
the note in [docs/LANGUAGE-SUPPORT.md](docs/LANGUAGE-SUPPORT.md#known-inconsistency-the-capability-gate)).

Editing, highlighting and every built-in provider work offline with no server at
all. Adding a language means implementing one trait and registering it — no
changes to the editor or the UI.

## ❯ Editing & workflow

The editor is the heart of Koda, and it is built for real projects.

- Multiple files in tabs, a lazy `.gitignore`-aware project tree, file
  create/rename/delete/duplicate/copy, git branch and per-file status, a
  changed-files list (`Ctrl+Shift+G`) where `Space` stages or unstages the
  selected file and `d` opens its diff, a colour-coded **diff panel** (also
  **Diff File** in the palette), and a **Commit Changes…** flow.
- A **split editor** (`Alt+V`): view two files side by side, with `Alt+O`
  moving focus between the panes. The focused pane owns the cursor and the
  active tab.
- Grouped undo, auto-pairing, smart newline, selection-aware indent/outdent,
  **detected indentation** (a two-space JavaScript file and a four-space Rust
  file both indent the way their project does, with no config), **multiple
  cursors** (`Ctrl+D` per occurrence, `Ctrl+Shift+L` for all, `Ctrl+Alt+↑`/`↓`
  to stack carets; typing and deletion apply at every cursor as one undo step),
  line move/duplicate, delete-line, go-to-matching-bracket, matching-bracket
  highlighting and a kill-ring with `Alt+Y` yank-pop.
- **Soft wrap** (`Alt+Z`): long lines wrap to the editor width instead of
  scrolling sideways. Up/Down move by visual row and keep a stable display
  column; character positions, selections, diagnostics, multiple cursors and
  undo are untouched.
- Quick open (`Ctrl+P`), inline fuzzy file filtering in the sidebar (`/`),
  find/replace with case, whole-word and regex options, project-wide text
  search (`Ctrl+Shift+F`), go-to-line, and **Revert File** to discard local
  edits.
- Project-wide symbol search, session persistence (open files, cursors and
  expanded folders come back next launch) and external-change detection.
- A background worker keeps detection, diagnostics, formatting and git off the
  UI thread.

## ✧ Look & feel

Koda has a personality, but the code always comes first.

- **The Mellow colours — the default.** Koda speaks the palette and semantic
  mappings of [Mellow][mellow], the separate named colorscheme shipped with
  Helix — not Helix's default theme. Blue keywords, bright-blue types, green
  strings, magenta numbers, grey italic comments.
- **Three bundled themes, one Settings screen.** Mellow is joined by
  **Midnight** (high-contrast dark) and **Daylight** (light), selectable at
  runtime. A small **Settings** screen (palette → **Settings**) also remembers
  line numbers, animations, soft wrap, indentation width, spaces vs tabs, inline
  diagnostics and automatic completion. Everything applies immediately, nothing
  needs a config file, and a fresh install is fully usable untouched.
- **Transparency first.** Plain surfaces use your terminal's own background;
  only the statusline, popups and the current line carry a soft panel.
- **A little familiar.** A star-cat keeps you company on the welcome screen and
  in empty states — never behind your code. It cycles through expressions, and
  the welcome screen offers five animated scenes (a starry night, a cozy desk, a
  rainy window, a drift of blossom, a quiet study).
- **One visual vocabulary.** `✦` stars, `☾` moons, `❯` pointers and `·`
  separators recur throughout, so the whole environment reads as one piece.
  Empty, loading and error states are all written in the same voice.
- **Quiet feedback.** Background results — a language server installing or
  recovering, a commit, a format — surface as short-lived toasts above the
  statusline, coloured by outcome, so you always know what Koda just did.

[mellow]: https://github.com/helix-editor/helix/blob/master/runtime/themes/mellow.toml

## ✦ Install

### Prebuilt binaries

Every tagged release publishes prebuilt archives on the
[Releases page](https://github.com/etokiyra/koda/releases). Pick your platform:

| Platform | Archive |
| --- | --- |
| Linux x86_64 (glibc) | `koda-<version>-linux-x86_64-gnu.tar.gz` |
| Linux x86_64 (musl) | `koda-<version>-linux-x86_64-musl.tar.gz` |
| macOS x86_64 (Intel) | `koda-<version>-macos-x86_64.tar.gz` |
| macOS arm64 (Apple silicon) | `koda-<version>-macos-arm64.tar.gz` |

Each archive contains the `koda` binary and the licence. Unpack it and put the
binary on your `PATH`:

```bash
tar -xzf koda-<version>-linux-x86_64-gnu.tar.gz
install -m 755 koda ~/.local/bin/koda   # or any directory on your PATH
koda .
```

SHA-256 checksums are published beside every archive (`<archive>.sha256`) and as
a combined `SHA256SUMS` file, so you can verify a download before running it:

```bash
sha256sum -c koda-<version>-linux-x86_64-gnu.tar.gz.sha256   # macOS: shasum -a 256 -c
```

Choose the archive that matches both your OS and your architecture; on Apple
silicon use `macos-arm64`, on Intel use `macos-x86_64`. Windows is not supported
— see [Platforms](#platforms).

### Build from source

Koda is a standard Cargo project (Rust edition 2024).

```bash
git clone https://github.com/etokiyra/koda.git
cd koda
cargo build --release
# binary: target/release/koda
```

Or install it onto your `PATH`:

```bash
cargo install --path .
```

During development:

```bash
cargo run -- .                 # open the current directory
cargo run -- src/main.rs       # open a file inside its detected project
```

### Platforms

**Linux and macOS are supported. Windows is not yet supported** — Koda has no
Windows CI, several provisioning plans require a Unix `sh`, and the code has not
been made correct for Windows path and launcher conventions. Do not expect a
working build on Windows.

Koda works on both glibc and musl Linux; the managed prebuilt bundles (clangd,
asm-lsp) are glibc-only and are withheld on musl, where a system toolchain or the
`cargo` fallback is used instead.

### System requirements

Koda itself needs only a terminal. Provisioning and git features shell out to a
small set of tools; each is used only when the feature needs it:

| Tool | Needed for | Optional? |
| --- | --- | --- |
| `curl` | every managed download | required to install managed tools |
| `tar` | unpacking `.tar.gz`/`.tar.xz` downloads | required for those downloads |
| `unzip`, `bsdtar` or `python3` | unpacking `.zip` downloads | any one of the three |
| `gpg` | verifying the Swift toolchain signature | **only for Swift** |
| `git` | branch, status, diff and commit features | optional; those features degrade to empty |
| `sh` | the Erlang/OTP `Install` script | required for that installer |
| `sha256sum`, `shasum`, `openssl` or `python3` | checking download hashes | any one of the four |

Editing, built-in language intelligence and local search work with no external
tool at all.

### Platform and verification

How each language was verified in the project-health audit. **`live`** means a
real language-server handshake (plus a completion or diagnostics round-trip where
noted) was run on that platform; **`tests`** means the built-in provider is
covered by the unit and render suite; **`skipped`** means the runtime or server
could not be installed here; **`untested`** means it was not exercised. CI is
configured for Linux (glibc), an Arch Linux container and Alpine (musl), and
macOS, but this audit did **not** observe a CI run, so those jobs are not counted
as verification. **Windows is not supported.**

| Language | Linux glibc (this host) | Linux musl | macOS | Windows |
| --- | --- | --- | --- | --- |
| Rust | tests | untested | untested | — |
| Go | live (`gopls` handshake + completion) | untested | untested | — |
| Python | tests | untested | untested | — |
| TypeScript / JavaScript | tests | untested | untested | — |
| C / C++ | live (`clangd` handshake + completion + diagnostics) | untested | untested | — |
| Java | tests (jdtls installed, not live-run) | untested | untested | — |
| C# | tests (OmniSharp installed, not live-run) | untested | untested | — |
| PHP | skipped (no PHP runtime) | untested | untested | — |
| Kotlin | live (`kotlin-language-server` handshake) | untested | untested | — |
| Lua | live (`lua-language-server` handshake) | untested | untested | — |
| Ruby | skipped (no Ruby or `gem`) | untested | untested | — |
| SQL | live (`sqls` handshake + completion) | untested | untested | — |
| Assembly | live (`asm-lsp` handshake) | untested | untested | — |
| Dart | live (`dart language-server` handshake) | untested | untested | — |
| Elixir | live (`ElixirLS` handshake) | untested | untested | — |
| Swift | skipped (needs `libxml2.so.2`, see below) | untested | untested | — |
| Perl | live (`PLS` handshake + completion; `Perl::LanguageServer` skipped) | untested | untested | — |
| HTML / CSS | live (both handshakes) | untested | untested | — |
| Markdown / JSON / TOML / YAML | tests (offline only) | untested | untested | — |

The Linux-glibc column was measured on the audit host (x86_64, Arch-based
EndeavourOS). The CI `uname -s && uname -m` step records the runner platform in
the workflow log, but no CI run was observed for this audit, so the macOS cells
remain unverified until that log is checked. Provisioning on musl is deliberately
limited — the glibc-only clangd and asm-lsp bundles are withheld there — and is
covered by the Alpine library-test job, not a live server. Swift was skipped
because this host lacks `libxml2.so.2` (Arch: `libxml2-legacy`); Koda reports that
package rather than downloading a toolchain that cannot run.

## ❯ Usage

```bash
koda              # start Koda in the current directory
koda .            # same, explicitly
koda src/main.rs  # start beside the file's project
```

Koda finds the nearest project root by walking up from the target through its
project markers (`Cargo.toml`, `go.mod`, `pyproject.toml`, `pom.xml`,
`Package.swift`, `mix.exs`, …) or a `.git` directory, establishes the language
context, and opens its welcome screen. A path you pass is offered there as an
**Open <path>** action rather than being opened for you.

## ✦ The welcome screen

Koda always opens on its home screen. It is a small, atmospheric scene — the
Koda familiar on a moonlit hill, at a cozy desk, by a rainy window, or in a
drift of blossom — above a keyboard-navigable menu. No file is opened
automatically, and nothing you passed on the command line is thrown away.

```text
              ✦         ·          ✧        ☾
           ·        ✧         ✦        ·
                 ✧        /\_/\              ✦
                         ( ･ω･ )
                          > ω <
           ·  ·  ·  ~~~~~~~~~~~~~~  ·  ·

                     ✦  K O D A  ✦
             your cozy little coding space

    ❯ Open a file…                          Ctrl+O
      Open a project…                choose a folder
      Create a new project…   Rust · Go · Python · C++
      Resume “my-project”            3 file(s)
      Keyboard shortcuts                       F1

      ↑↓ choose  ·  Enter open  ·  v scene  ·  Ctrl+Shift+P
```

- **Move** with `↑`/`↓` (or `Home`/`End`), **open** with `Enter`.
- **Open a file…** or press `Ctrl+O`; **Open a project…** browses directories.
- **Cycle the scene** with `v`, or **Change Welcome Scene** in the palette.
- **Toggle Animations** in the palette freezes the scene and spinner for a calm,
  reduced-motion experience.
- **Resume** restores the workspace's saved session; **recent projects and
  files** reappear here once you have opened a few.
- Any path given on the command line shows up as an **Open <path>** row.
- **Keyboard shortcuts** (`F1`) opens the cheatsheet, and `Ctrl+Shift+P` opens
  the command palette.

Closing the last tab — or running **Welcome Screen** from the palette — brings
the home screen back.

### Create a new project

**Create a new project…** is a guided, three-step flow:

1. **Choose a folder** — browse with the arrows, `Enter` opens a directory or
   confirms the highlighted one, `←` goes up, `Esc` cancels.
2. **Name it** — the name is validated for your platform and checked against
   the chosen folder so an existing project is never overwritten.
3. **Pick a language** — Rust, Go, Python, TypeScript, JavaScript, Java, C#,
   PHP, Kotlin, Lua, Ruby, Elixir, Dart, Swift, Perl, SQL, Assembly, HTML,
   Shell, C or C++, each shown with what Koda will generate. `Esc` steps back
   at any point.

Koda then scaffolds the project without a network connection or a toolchain and
opens it, ready to edit:

| Language | Generated |
| --- | --- |
| **Rust** | `Cargo.toml`, `src/main.rs`, `.gitignore` |
| **Go** | `go.mod`, `main.go` |
| **Python** | `pyproject.toml`, `src/<package>/__init__.py` and `__main__.py` |
| **TypeScript** | `package.json`, `tsconfig.json`, `src/index.ts`, `.gitignore` |
| **JavaScript** | `package.json`, `src/index.js`, `.gitignore` |
| **Java** | `pom.xml`, `src/main/java/…/App.java`, `.gitignore` |
| **C#** | a `*.csproj`, `Program.cs`, `.gitignore` |
| **PHP** | `composer.json`, `index.php`, `.gitignore` |
| **Lua** | `init.lua`, `.luarc.json`, `.gitignore` |
| **Kotlin** | `settings.gradle.kts`, `build.gradle.kts`, `src/main/kotlin/Main.kt`, `.gitignore` |
| **Ruby** | `Gemfile`, `lib/<name>.rb`, `.gitignore` |
| **Elixir** | `mix.exs`, `lib/<name>.ex`, `.gitignore` |
| **Dart** | `pubspec.yaml`, `bin/main.dart`, `.gitignore` |
| **Swift** | `Package.swift`, `Sources/<name>/main.swift`, `.gitignore` |
| **Perl** | an executable `<name>.pl` (plus a `README.md`) |
| **SQL** | `schema.sql`, `README.md` |
| **Assembly** | `main.asm`, `Makefile`, `.gitignore` |
| **HTML** | `index.html` and `style.css` |
| **Shell** | an executable `<name>.sh` (plus a `README.md`) |
| **C** | `CMakeLists.txt`, `src/main.c`, `.gitignore` |
| **C++** | `CMakeLists.txt`, `src/main.cpp`, `.gitignore` |

If generation fails partway through, the partial folder is left untouched and
Koda reports what happened rather than deleting anything.


## ☾ Keyboard shortcuts

<details>
<summary>Expand the full keymap</summary>

<!-- keymap:start -->
| Category | Shortcut | Action |
| --- | --- | --- |
| File | `Ctrl+S` | Save |
| File | `Ctrl+Shift+S` | Save As… |
| File | `Ctrl+O` | Open File… |
| File | `Ctrl+P` | Quick Open… |
| File | `Ctrl+W` | Close Tab |
| File | `Ctrl+N` | New File… |
| File | `Ctrl+Q` | Quit |
| Edit | `Ctrl+Z` | Undo |
| Edit | `Ctrl+Shift+Z` | Redo |
| Edit | `Ctrl+A` | Select All |
| Edit | `Ctrl+D` | Select Next Occurrence |
| Edit | `Ctrl+Shift+L` | Select All Occurrences |
| Edit | `Ctrl+Alt+↓` | Add Cursor Below |
| Edit | `Ctrl+Alt+↑` | Add Cursor Above |
| Edit | `Ctrl+C` | Copy |
| Edit | `Ctrl+X` | Cut |
| Edit | `Ctrl+V` | Paste |
| Edit | `Alt+Y` | Yank Pop |
| Edit | `Ctrl+Space` | Complete |
| Edit | `Ctrl+F` | Find |
| Edit | `Ctrl+H` | Replace |
| Edit | `Alt+Enter` | Replace All |
| Edit | `Ctrl+G` | Go to Line… |
| Edit | `Ctrl+/` | Toggle Comment |
| Edit | `Tab` | Indent |
| Edit | `Shift+Tab` | Outdent |
| Edit | `Alt+↑` | Move Line Up |
| Edit | `Alt+↓` | Move Line Down |
| Edit | `Ctrl+Shift+D` | Duplicate Line |
| Edit | `Ctrl+Shift+K` | Delete Line |
| Edit | `Alt+M` | Go to Matching Bracket |
| Language | `Ctrl+Shift+I` | Format Document |
| Language | `Ctrl+Shift+H` | Hover |
| Language | `F12` | Go to Definition |
| Language | `Shift+F12` | Find References |
| Language | `Ctrl+Shift+O` | Go to Symbol… |
| Language | `F2` | Rename Symbol |
| Language | `Ctrl+.` | Code Actions |
| Diagnostics | `F8` | Next Diagnostic |
| Diagnostics | `Shift+F8` | Previous Diagnostic |
| Diagnostics | `Ctrl+Shift+M` | Show Diagnostics |
| Project | `Ctrl+T` | Go to Symbol in Workspace… |
| Project | `Ctrl+Shift+F` | Search in Project… |
| Git | `Ctrl+Shift+G` | Changed Files… |
| Git | `Alt+D` | Diff File |
| View | `Ctrl+B` | File Panel: Focus / Hide |
| View | `Ctrl+E` | Focus File Tree / Editor |
| View | `Alt+I` | Toggle Inline Diagnostics |
| View | `Alt+Z` | Toggle Soft Wrap |
| View | `F5` | Refresh File Tree |
| View | `Alt+V` | Split Editor |
| View | `Alt+O` | Focus Other Pane |
| View | `/` | Filter File Tree |
| View | `Ctrl+Tab` | Next Tab |
| View | `Ctrl+Shift+Tab` | Previous Tab |
| View | `Ctrl+Shift+P` | Command Palette |
| Help | `F1` | Keyboard Shortcuts |
| Editor | `Arrows` | move the cursor |
| Editor | `Ctrl+←/→` | move by word |
| Editor | `Shift+Arrows` | select |
| Editor | `Home / End` | line start / end |
| Editor | `Ctrl+Home / End` | document start / end |
| Editor | `PageUp / PageDown` | scroll a page |
| Editor | `F3 / Shift+F3` | find next / previous |
| Editor | `Ctrl+PageUp/Down` | previous / next tab |
| Context | `Alt+C / Alt+W / Alt+R` | find: toggle case / whole word / regex |
| Context | `Alt+Enter` | find: replace every match |
| Context | `Space` | changed files: stage or unstage |
| Context | `d` | changed files: show the diff |
| Context | `/` | file tree: filter project files |
| Context | `.` | file tree: toggle hidden files |
| Context | `↑ / ↓` | welcome: choose an action |
| Context | `Enter` | welcome: open the chosen action |
| Context | `v` | welcome: cycle the animated scene |
<!-- keymap:end -->

Editor keys behave as you would expect: arrows, `Home`/`End`, `PageUp`/`PageDown`,
`Shift`+arrows to select, `Ctrl`+arrows for word movement. Typing with a
selection replaces it, brackets and quotes pair up, and consecutive typing
undoes as one step.

</details>

## ✦ Architecture

The full source tree, dependency rules, the detection-signal model and how to
add a language live in **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**.

The guiding rule: **language-specific logic never leaks into the editor or the
UI.** Detection answers *what is this file*; providers answer *how do we support
it*. Everything expensive runs on a background worker so the UI never blocks.

## ☾ Language detection

Extensions are only one signal. File-level evidence decides *what a file is*;
project context corroborates it:

1. **File name** — special names like `Makefile`, `Dockerfile`
2. **Shebang** — `#!/usr/bin/env …`
3. **File extension** — `.rs`, `.go`, …
4. **Content analysis** — distinctive syntax, kept lightweight
5. **Project context** — `Cargo.toml`, `go.mod`, … (a confidence boost when it
   agrees with the file's own signals)

A project marker never overrides a file's own nature: `README.md` inside a Rust
project stays plain text, while `main.rs` becomes *high* confidence Rust — and
the workspace still understands that it lives inside a Rust project.

## ❯ Development

```bash
cargo test                    # unit + rendering tests
cargo clippy --all-targets    # lints (must be clean)
cargo fmt                     # formatting
cargo run -- .                # try it
```

Contributions are developed commit-by-commit: small, focused, compiling and
tested. See [`docs/PRODUCT.md`](docs/PRODUCT.md) for the product direction,
[`ROADMAP.md`](ROADMAP.md) for where this is going and
[`CHANGELOG.md`](CHANGELOG.md) for what has changed.

## ✦ Credits & license

- Colour language: **[Mellow][mellow]**, the named colorscheme shipped with
  [Helix](https://helix-editor.com), by Rohit K Viswanath. Koda translates its
  palette and semantic mappings into its own interface; the layout, components
  and ASCII art are Koda's.
- Built on the shoulders of [`ratatui`](https://ratatui.rs),
  [`crossterm`](https://github.com/crossterm-rs/crossterm),
  [`ropey`](https://github.com/cessen/ropey) and
  [`serde_json`](https://github.com/serde-rs/json).

Licensed under the [Mozilla Public License 2.0](LICENSE).

<p align="center"><sub>✦ &nbsp; made with care &nbsp; ☾</sub></p>
