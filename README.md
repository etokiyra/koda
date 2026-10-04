# Koda

<p align="center"><em>A modern, lightweight, terminal-native IDE.</em></p>

<p align="center">
  <a href="LICENSE"><img alt="License: GPLv3" src="https://img.shields.io/badge/license-GPLv3-90b99f?style=flat-square"></a>
  <img alt="Rust 2024" src="https://img.shields.io/badge/rust-2024-ea83a5?style=flat-square&logo=rust&logoColor=white">
  <img alt="Tests" src="https://img.shields.io/badge/tests-536%20passing-9dc6ac?style=flat-square">
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

Missing something? Koda notices when a project or a supported file is open
without its language server and offers to install it, once, without blocking
startup. Opening a `Cargo.toml` project offers Rust tooling before any file is
opened, and the same holds for every language Koda can provision. Most
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
**Language Setup…** in the command palette then lists every tool Koda knows
about and installs a missing one with a single `Enter` — through the official
channel, so provenance and integrity stay with the package manager:

| Tool | Purpose | Koda installs it with |
| --- | --- | --- |
| `rust-analyzer` | Rust language server | `rustup component add rust-analyzer` |
| `gopls` | Go language server | `go install`, with the **official Go toolchain** provisioned if missing |
| `pylsp` | Python language server | a Koda-managed virtualenv (or `pipx`/`uv`/`pip --user`) |
| `bash-language-server` | Shell language server | `npm`, with a prefix Koda manages |
| `typescript-language-server` | TypeScript & JavaScript language server | `npm`, with a prefix Koda manages |
| `clangd` | C/C++ language server | the official **self-contained clangd bundle**; detected if installed |
| `jdtls` | Java language server | a managed Adoptium JDK 25 + Eclipse JDT |
| `omnisharp` | C# language server | a managed .NET SDK + OmniSharp |
| `phpactor` | PHP language server | the official **`phpactor.phar`** (needs a PHP runtime) |
| `lua-language-server` | Lua language server | a self-contained download managed by Koda |
| `kotlin-language-server` | Kotlin language server | a managed Adoptium JDK 21 + the pinned server release |
| `sqls` | SQL language server | `go install`, with the **official Go toolchain** provisioned if missing |
| `solargraph` | Ruby language server | `gem install` into a **Koda-private gem home** (needs Ruby) |
| `asm-lsp` | Assembly language server | the **prebuilt release**; `cargo`/`rustup` fallback |
| `sourcekit-lsp` | Swift language server | a **GPG-verified Swift toolchain** (native or the portable UBI10 build plus a compatibility layer); detected otherwise |
| `dart` analysis server | Dart/Flutter language server | a **managed Dart SDK** (Google `dart-archive`, checksum-verified) |
| `elixir-ls` / `language_server.sh` | Elixir language server | a managed **Erlang/OTP + Elixir + ElixirLS** stack (`builds.hex.pm`, checksum-verified) |
| `pls` | Perl language server (preferred) | a checksum-verified `cpanm` into a Koda-managed `local::lib` |
| `Perl::LanguageServer` | Perl language server (fallback) | the same isolated `local::lib` (needs `Coro`, so on older Perls) |
| `vscode-html-language-server` | HTML language server | `npm` (`vscode-langservers-extracted`), Koda-managed prefix |
| `vscode-css-language-server` | CSS language server | `npm` (`vscode-langservers-extracted`), Koda-managed prefix |
| `rustfmt` | Rust formatting | `rustup component add rustfmt` |
| `gofmt` | Go formatting | ships with the Go toolchain |
| `prettier` | Web/HTML/CSS/JSON/YAML/Markdown formatting | `npm install -g prettier` (Koda-managed Node/prefix) |
| `clang-format` | C/C++ formatting | ships with the Clang/LLVM toolchain |
| `shfmt` | Shell formatting | `go install`, with the official Go toolchain provisioned if missing |
| `perltidy` | Perl formatting | `cpan Perl::Tidy` |
| `dart format` | Dart formatting | ships with the managed Dart SDK (via a temporary-file contract) |
| `mix format` | Elixir formatting | ships with the managed Erlang/Elixir runtime (stdin contract) |

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
| Go toolchain | `go.dev/dl` | SHA-256 from the release JSON, fail-closed | Linux/macOS/Windows, x86_64/arm64 |
| clangd | `clangd/clangd` release | SHA-256 asset digest | Linux/macOS (glibc); not musl |
| asm-lsp | `bergercookie/asm-lsp` release (prebuilt) | SHA-256 asset digest | Linux x86_64, macOS x86_64/arm64; `cargo`/`rustup` elsewhere |
| phpactor | `phpactor/phpactor` release (`phpactor.phar`) | SHA-256 asset digest | wherever a system `php` exists |
| Dart SDK | Google `dart-archive` | sibling `.sha256sum`, fail-closed | Linux/macOS/Windows, x86_64/arm64 |
| Node.js | `nodejs.org` | `SHASUMS256.txt` | Linux/macOS/Windows, x86_64/arm64 |
| Eclipse Adoptium JDK | `api.adoptium.net` | checksum from the API | Linux/macOS/Windows |
| Java (jdtls) | Eclipse snapshots + managed JDK 25 | — | with a managed JDK |
| Kotlin | pinned server + managed JDK 21 | — | with a managed JDK |
| Lua | `LuaLS` release | — | Linux/macOS/Windows |
| OmniSharp | GitHub release + `dotnet-install.sh` | — | Linux/macOS/Windows |
| Perl (PLS, and `Perl::LanguageServer`) | CPAN, via a checksum-verified `cpanm` | SHA-256 (`App::cpanminus`), fail-closed | wherever a system `perl` exists |
| solargraph | RubyGems, into a Koda-private gem home | RubyGems signatures/HTTPS | wherever a system `gem` exists |

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

### Language setup status

How Koda obtains each language server, based on the actual implementation (not
marketing). "Automatic" means Koda downloads and verifies the tooling itself;
"needs X" means Koda installs through an existing toolchain and reports the
prerequisite when it is absent.

| Language | Language server | Koda's setup |
| --- | --- | --- |
| Rust | `rust-analyzer` | automatic (`rustup`, bootstrapped if missing) |
| Go | `gopls` | automatic (`go install`, provisioning official Go if missing) |
| Python | `pylsp` | automatic (Koda-managed virtualenv) |
| TypeScript / JavaScript | `typescript-language-server` | automatic (Koda-managed Node.js) |
| C / C++ | `clangd` | automatic (official self-contained clangd bundle; glibc Linux and macOS) |
| Java | `jdtls` | automatic (checksum-verified JDK 25) |
| C# | `OmniSharp` | automatic (managed .NET SDK) |
| PHP | `phpactor` | automatic `phpactor.phar` (needs a PHP runtime) |
| Kotlin | `kotlin-language-server` | automatic (dedicated JDK 21) |
| Lua | `lua-language-server` | automatic (self-contained archive) |
| Ruby | `solargraph` | automatic into an isolated gem home (needs Ruby) |
| SQL | `sqls` | automatic (`go install`, provisioning official Go if missing) |
| Assembly | `asm-lsp` | automatic (prebuilt release; `cargo`/`rustup` fallback) |
| Swift | `sourcekit-lsp` | automatic (GPG-verified toolchain; native or portable + compatibility layer) |
| Dart / Flutter | `dart language-server` | automatic (checksum-verified Dart SDK) |
| Elixir | `ElixirLS` | automatic (Erlang/OTP + Elixir + ElixirLS) |
| Perl | `PLS` (or `Perl::LanguageServer`) | automatic (checksum-verified `cpanm` + isolated `local::lib`) |
| HTML | `vscode-html-language-server` | automatic (Koda-managed Node.js) |
| CSS | `vscode-css-language-server` | automatic (Koda-managed Node.js) |
| Markdown / JSON / TOML / YAML | — | offline built-in intelligence, no server needed |

### Supported languages

| Language | Syntax | Diagnostics | Symbols | Completion | Hover | Navigation | Rename |
| --- | --- | --- | --- | --- | --- | --- | --- |
| **Rust** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **Go** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **Python** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **TypeScript** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **JavaScript** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **C** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **C++** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **Java** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **C#** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **PHP** | built-in | built-in | built-in | built-in | built-in | built-in | LSP |
| **Kotlin** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **HTML** | built-in | built-in + LSP | built-in (ids) | built-in | built-in | LSP | LSP |
| **CSS** | built-in | built-in + LSP | built-in (selectors) | built-in | built-in | LSP | LSP |
| **Lua** | built-in | built-in + LSP | built-in | built-in + LSP | built-in + LSP | built-in + LSP | LSP |
| **Ruby** | built-in | built-in | built-in | built-in | built-in | built-in | LSP |
| **SQL** | built-in | built-in | built-in | built-in | built-in | built-in | LSP |
| **Perl** | built-in | built-in | built-in | built-in | built-in | built-in | LSP |
| **Dart** | built-in | built-in | built-in | built-in | built-in | built-in | LSP |
| **Elixir** | built-in | built-in | built-in | built-in | built-in | built-in | LSP |
| **Swift** | built-in | built-in | built-in | built-in | built-in | built-in | LSP |
| **Assembly** | built-in | — | built-in (labels) | built-in | built-in | built-in | — |
| **Shell** | built-in | LSP | built-in (functions) | built-in | built-in | built-in | — |
| **Markdown** | built-in | — | built-in (headings) | — | — | — | — |
| **JSON** | built-in | built-in | built-in (top-level keys) | built-in (literals) | — | — | — |
| **TOML** | built-in | built-in | built-in (tables & keys) | built-in (literals) | — | — | — |
| **YAML** | built-in | built-in | built-in (top-level keys) | built-in (literals) | — | — | — |

Prose and configuration files are first-class too. Markdown, JSON, TOML and YAML
get syntax highlighting, structural diagnostics where they make sense, and a
symbol outline, all offline and with no setup. **Python**, **Shell**,
**TypeScript**, **JavaScript**, **C**, **C++**, **Java**, **C#**, **PHP**,
**Kotlin**, **Lua**, **Ruby**, **SQL**, **Perl**, **Dart**, **Elixir**,
**Swift**, **Assembly**, **HTML** and **CSS** work offline through Koda's
built-in intelligence (highlighting, diagnostics where available, symbols,
completion, hover, navigation) and gain richer server-backed features when Koda
installs or finds a managed language server. Adding a language means implementing
one trait and registering it — no changes to the editor or the UI.

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

- **The Mellow colours.** Koda speaks the palette and semantic mappings of
  [Mellow][mellow], the separate named colorscheme shipped with Helix — not
  Helix's default theme. Blue keywords, bright-blue types, green strings,
  magenta numbers, grey italic comments.
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

## ❯ Usage

```bash
koda              # start Koda in the current directory
koda .            # same, explicitly
koda src/main.rs  # start beside the file's project
```

Koda finds the project root (`Cargo.toml`, `go.mod`, or a `.git` directory),
establishes the language context, and opens its welcome screen. A path you pass
is offered there as an **Open <path>** action rather than being opened for you.

## ✦ The welcome screen

Koda always opens on its home screen. It is a small, atmospheric scene — the
Koda familiar on a moonlit hill, at a cozy desk, by a rainy window, or in a
drift of blossom — above a keyboard-navigable menu. No file is opened
automatically, and nothing you passed on the command line is thrown away.

```text
              ✦         ·          ✧        ☾
           ·        ✧         ✦        ·
                 ✧        /\___/\              ✦
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

| Shortcut | Action |
| --- | --- |
| `Ctrl+S` | Save |
| `Ctrl+Shift+S` | Save as |
| `Ctrl+Q` | Quit (press twice if there are unsaved changes) |
| `Ctrl+O` | Open file (path prompt) |
| `Ctrl+N` | New file |
| `Ctrl+P` | Quick open |
| `Ctrl+Shift+P` | Command palette |
| `F1` | Keyboard-shortcuts cheatsheet |
| `F5` | Refresh the file tree and git status |
| `↑` / `↓` (welcome) | Choose a welcome-screen action |
| `Enter` (welcome) | Open the chosen action |
| `v` (welcome) | Cycle the animated scene |
| `Ctrl+F` / `Ctrl+H` | Find / replace |
| `F3` / `Shift+F3` | Find next / previous |
| `Ctrl+Shift+F` | Search in the whole project |
| `Alt+C` / `Alt+W` / `Alt+R` (in find) | Toggle case / whole word / regex |
| `Alt+Enter` (in replace) | Replace every match |
| `Ctrl+G` | Go to line |
| `Ctrl+Shift+G` | List the files changed in git |
| `Space` (in changed files) | Stage / unstage the selected file |
| `d` (in changed files) | Show the selected file's diff |
| `Alt+D` | Show the active file's unified diff |
| `Alt+I` | Toggle inline diagnostic messages |
| `Alt+Z` | Toggle soft wrap |
| `Ctrl+Shift+M` | Show diagnostics |
| `Ctrl+Z` / `Ctrl+Shift+Z` | Undo / redo |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy / cut / paste |
| `Alt+Y` | Yank-pop: replace the last paste with an earlier kill |
| `Ctrl+Space` | Complete (manual; suggestions also appear as you type) |
| `Ctrl+Shift+I` | Format the active file |
| `Ctrl+Shift+H` | Hover: info about the symbol under the cursor |
| `Ctrl+A` | Select all |
| `Ctrl+D` | Add a cursor at the next occurrence of the selection |
| `Ctrl+Shift+L` | Add a cursor at every occurrence |
| `Ctrl+Alt+↓` / `Ctrl+Alt+↑` | Add a cursor on the line below / above |
| `Esc` | End a multi-cursor session, then clear the selection |
| `Ctrl+B` | Focus the file panel, or hide it when focused |
| `Ctrl+E` | Toggle focus between the file tree and editor |
| `Alt+V` | Split the editor into two panes |
| `Alt+O` | Focus the other pane |
| `Ctrl+W` | Close tab (press twice to discard unsaved changes) |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Ctrl+PageDown` / `Ctrl+PageUp` | Next / previous tab |
| `Ctrl+/` | Toggle comment |
| `F8` / `Shift+F8` | Next / previous diagnostic |
| `Ctrl+Shift+O` | Go to symbol in the active file |
| `Ctrl+T` | Go to symbol in the workspace |
| `F12` / `Shift+F12` | Go to definition / find references |
| `F2` | Rename symbol |
| `Ctrl+.` | Code actions |
| `Tab` / `Shift+Tab` | Indent / outdent selection |
| `Alt+↑` / `Alt+↓` | Move line up / down |
| `Ctrl+Shift+D` | Duplicate line |
| `Ctrl+Shift+K` | Delete line |
| `Alt+M` | Go to matching bracket |
| `/` (in the tree) | Filter project files |
| `.` (in the tree) | Toggle hidden files |

Editor keys behave as you would expect: arrows, `Home`/`End`, `PageUp`/`PageDown`,
`Shift`+arrows to select, `Ctrl`+arrows for word movement. Typing with a
selection replaces it, brackets and quotes pair up, and consecutive typing
undoes as one step.

</details>

## ✦ Architecture

<details>
<summary>Expand the source tree</summary>

```text
src/
├── app/          # state, event loop, overlays, command dispatch
├── commands/     # the extensible command registry
├── editor/       # buffers, documents, cursor, history (no language logic)
├── filesystem/   # thin, well-behaved fs helpers
├── git/          # lightweight git integration
├── language/
│   ├── detection/  # signals, scoring, confidence
│   ├── provider/   # the LanguageProvider trait + registry
│   ├── lsp/        # JSON-RPC client, lifecycle, result conversion
│   ├── tools/      # discovery + trusted provisioning
│   ├── data.rs     # shared scanners for JSON/TOML/YAML
│   ├── rust/       # Rust provider
│   ├── go/          # Go provider
│   ├── python/     # Python provider (built-in, offline)
│   ├── shell/      # Shell provider (bash/zsh/sh)
│   ├── web/        # TypeScript/JavaScript provider
│   ├── c/          # C/C++ provider
│   ├── java/       # Java provider (built-in, offline)
│   ├── csharp/     # C# provider (built-in, offline)
│   ├── php/        # PHP provider (built-in, offline)
│   ├── kotlin/     # Kotlin provider (managed JDK 21 LSP)
│   ├── lua/        # Lua provider (managed self-contained LSP)
│   ├── sql/        # SQL provider (dialect-neutral)
│   ├── ruby/       # Ruby provider
│   ├── perl/       # Perl provider
│   ├── asm/        # Assembly provider (x86/AArch64 baseline)
│   ├── dart/       # Dart provider
│   ├── elixir/     # Elixir provider
│   ├── swift/      # Swift provider
│   ├── html/       # HTML provider (tags, ids)
│   ├── css/        # CSS provider (selectors, properties)
│   ├── markdown/   # Markdown provider (headings as symbols)
│   └── json|toml|yaml/  # configuration-data providers
├── project/      # workspace, project detection, file tree, templates
├── recent.rs     # recent projects and files for the welcome screen
├── regex.rs      # a small regex engine for search
├── search.rs     # project-wide text search
├── session.rs    # per-project session persistence
├── terminal/     # terminal lifecycle + OSC 52 clipboard
├── background.rs # worker thread for detection, LSP, git and formatting
└── ui/           # rendering for every surface
```

The guiding rule: **language-specific logic never leaks into the editor or the
UI.** Detection answers *what is this file*; providers answer *how do we support
it*. Everything expensive runs on a background worker so the UI never blocks.

</details>

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
tested. See [`ROADMAP.md`](ROADMAP.md) for where this is going and
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

Licensed under the [GNU General Public License v3.0](LICENSE).

<p align="center"><sub>✦ &nbsp; made with care &nbsp; ☾</sub></p>
