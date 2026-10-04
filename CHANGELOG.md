# Changelog

All notable changes to Koda are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Multi-line templates, JSX tags and embedded `<script>`/`<style>` highlighting

- **Template literals now span lines.** The web tokenizer carries a template
  state between lines, so a backtick string stays a string, `${ … }`
  interpolations resume code highlighting (with brace balancing for nested
  objects) and return to template text when they close.
- **JSX tags span lines.** An open `<Component` whose attributes continue on the
  next line keeps its attribute highlighting until the closing `>`, carried by a
  `JsxTag` lexical mode.
- **HTML embeds its sub-languages.** A `<script>` body is tokenized as
  JavaScript and a `<style>` body as CSS, across lines, with the embedded
  tokenizer's own multi-line state; the closing tag hands the rest of the line
  back to HTML. A self-closing `<script />` has no body.
- **Tag-balance diagnostics ignore raw bodies.** `<` and `>` inside a
  `<script>`/`<style>` element are code, so the HTML tag-balance check no longer
  reports false "never closed" tags for comparisons like `a < b`.
- `HighlightState` grew a small, shared lexical vocabulary (`LexMode` for
  templates, `Embed` for HTML regions) alongside the existing block-comment flag,
  so the carry-over is explicit rather than provider-specific.

### Managed Go, clangd, asm-lsp and phpactor — fewer system prerequisites

- **Go is fully automatic, and it unlocks three tools at once.** Koda resolves
  the current stable Go release from `go.dev/dl/?mode=json` (which publishes the
  version and the SHA-256 of every archive together, verified fail-closed),
  extracts it under `tools/go`, and installs tooling into a **private
  `GOPATH`/`GOBIN`** so nothing lands in the user's `~/go`. A system Go is reused
  when present; otherwise the official toolchain is provisioned. `gopls`, `sqls`
  and `shfmt` now need no system Go at all. **Live-verified**: `gopls` installed
  with no system Go on `PATH`, and its LSP handshake.
- **clangd** is provisioned from the official, self-contained `clangd` release
  (a ~120 MB bundle with its own resource headers) instead of requiring a full
  LLVM toolchain; the GitHub asset digest is verified. **Live-verified**: managed
  clangd installed and completed a full `initialize` handshake.
- **asm-lsp** now prefers the project's prebuilt release (verified digest) over a
  `cargo` build; `cargo install` remains the fallback, and it now bootstraps the
  Rust toolchain via the official `rustup` installer when `cargo` is missing.
- **phpactor** is provisioned as the official `phpactor.phar` (verified digest,
  made executable) so Composer is no longer required — only a PHP runtime.
- **Ruby** gets an isolated gem home: `solargraph` installs under Koda's data
  directory instead of the user's global gems, with `GEM_HOME`/`GEM_PATH` scoped
  to the server process.
- **musl safety.** The prebuilt `clangd`/`asm-lsp` bundles are glibc-linked, so
  Koda does not offer them on musl (where the system toolchain or the `cargo`
  fallback is used); the Go toolchain and its tools are static and remain
  available.

### Portable Swift, a working Perl server on modern Perl, and `mix format`

- **Swift now runs on distributions swift.org does not build for, including
  Arch.** On a supported distribution Koda still installs the native artifact; on
  any other glibc Linux system it installs the portable **UBI10** build (the same
  one the maintained Arch package uses) plus a **managed compatibility layer** —
  a `libncurses.so.6` alias to the system's `libncursesw.so.6` and a link to the
  system's `libxml2.so.2` — used through a scoped `LD_LIBRARY_PATH`. Nothing in
  the system is copied or modified. When the distribution's `libxml2.so.2` is
  missing, Koda names the package to install (Arch: `libxml2-legacy`) instead of
  downloading a toolchain that cannot run. The download is GPG-verified against
  swift.org's keys and is reused when it already verifies, so a retried install
  does not re-download it. **Live-verified on Arch**: UBI10 install, compatible
  launch, and the LSP handshake.
- **Perl works on current Perl again.** `Perl::LanguageServer` depends on `Coro`,
  whose latest release does not compile on Perl ≥ 5.41. Koda now prefers **PLS**
  (a maintained, Coro-free Perl language server) — installed from CPAN with a
  checksum-verified `cpanm` into the same isolated `local::lib` — and keeps
  `Perl::LanguageServer` as a discovered fallback. A language can now have more
  than one candidate server and Koda picks the first that is actually available,
  in preference order. **Live-verified**: PLS `initialize` handshake and
  completion/definition/symbol capabilities on Perl 5.42.
- **Elixir formatting via `mix format`.** The Elixir provider exposes
  `Formatting` and formats through `mix format -` (stdin → stdout) with Koda's
  managed Erlang/Elixir and a private `MIX_HOME`, run from the enclosing Mix
  project so `.formatter.exs` applies. **Live-verified** with the managed runtime.
- **Shared reliability.** Downloads now retry transient network failures
  (`--retry`) and reuse an already-verified GPG archive; libc is detected
  explicitly (musl is reported unsupported rather than attempted); a
  user-writable library directory lets a rootless user supply a compatibility
  library.

### Managed Swift, Elixir and Perl — every discovery-only language made first-class

- **Swift is now managed on supported systems.** On a distribution swift.org
  builds for (Ubuntu 22.04/24.04/26.04, Debian 12/13, Fedora 39/41, Amazon Linux
  2/2023, mapped through `/etc/os-release` including derivatives) Koda resolves
  the exact toolchain artifact, downloads it, **verifies swift.org's detached GPG
  signature against `https://www.swift.org/keys/all-keys.asc` in an isolated
  keyring (fail-closed)**, and extracts it under `tools/swift`. `sourcekit-lsp`
  is then discovered and launched with a scoped `PATH`. The download is large
  (~1.1 GB, ~3.5 GB extracted) and Koda shows that estimate before offering it.
  On a distribution swift.org does not build for (for example Arch, whose ABI
  differs), Koda keeps discovery and built-in editing and explains the exact
  limitation instead of downloading a toolchain that cannot run.
- **Elixir is now a fully provisioned stack.** Koda installs a compatible
  **Erlang/OTP + Elixir** pair from the Erlang Ecosystem Foundation's
  `builds.hex.pm` (the service behind the official `setup-beam` action),
  checksum-verified via its `builds.txt`, runs the OTP `Install -minimal` pass,
  fetches the official **ElixirLS** release (GitHub asset digest verified) and
  builds it once with the managed runtime and a **private `MIX_HOME`/`HEX_HOME`**
  so `~/.mix` is never touched. ElixirLS's `language_server.sh` is discovered and
  launched with the managed runtimes on a scoped `PATH`. Works on glibc systems
  even where `bob` has no explicit build (best-effort Ubuntu 24.04 target,
  excluded on musl). **Live-verified on this host**: OTP + Elixir runtime and the
  ElixirLS language server.
- **Perl now bootstraps its own installer.** Koda downloads a **checksum-verified
  `App::cpanminus`** archive (so an interactive, unconfigured `cpan` is never run)
  and installs `Perl::LanguageServer` with `--local-lib <tools/perl5>` — the
  system Perl is never modified. The server is launched through its real module
  invocation (`perl -MPerl::LanguageServer -e Perl::LanguageServer->run`) with
  `PERL5LIB`/`PATH` scoped to the managed library. A known upstream limitation is
  reported honestly: `Perl::LanguageServer` depends on `Coro`, whose latest
  release (6.57, 2020) does not compile on Perl ≥ 5.41.
- **Shared installer architecture.** New signature-verified (`DownloadGpg`) and
  GitHub-digest (`GithubRelease`) download steps, a `BobBuild` step for
  checksum-verified hex.pm runtimes, install commands with per-command environment
  and working directory, resilient downloads (stall detection, long-transfer
  timeout), a **private GPG keyring**, and a real failure reason when a verified
  install does not run (for example a missing shared library).
- **Setup UX.** Language Setup now distinguishes what Koda can install, what
  needs a missing prerequisite, and what the platform does not support, and shows
  an approximate download size before a large managed install.

### Managed Dart SDK, isolated Perl server, and the Swift assessment

- **Dart is now fully automatic.** Koda provisions its own Dart SDK on demand
  from Google's official `dart-archive`, verifying the sibling `.sha256sum`
  (fail-closed) and extracting under `tools/dart-sdk`. It is discovered through
  the managed bin directory and launched as `dart language-server`, giving
  diagnostics, completion, hover, navigation, symbols, rename and code actions.
  Only Dart is downloaded — never Flutter, because the analysis server is part
  of Dart. `dart format` is integrated through a **safe temporary-file contract**
  (`--output=show --summary=none`, since it does not read stdin). **Live-verified
  on this host**: checksum, extraction, discovery, LSP `initialize` handshake and
  formatting.
- **Perl has a real, isolated installer.** `Perl::LanguageServer` ships no
  executable; the real launch is
  `perl -MPerl::LanguageServer -e Perl::LanguageServer->run` over stdio. Koda
  can install the module with `cpanm --local-lib <tools/perl5> --notest` into an
  isolated `local::lib` (never the system Perl), and launches it with `PERL5LIB`
  and `PATH` scoped to that library. `cpanm` is the only prerequisite; without
  it Koda discovers an existing server and explains the requirement.
- **ElixirLS discovery broadened.** The tool prober now tries alternate launcher
  names, so an ElixirLS release's `language_server.sh` is found alongside the
  `elixir-ls` binary packaged by Homebrew/Mason/AUR.
- **Swift assessment (deferred, documented).** The official Linux toolchains are
  large (Swift 6.4.0 Ubuntu 24.04 is ~1.12 GB compressed, ~2.5 GB extracted) and
  are only published for specific Ubuntu/Debian/Fedora/Amazon/RHEL releases;
  swift.org distributes GPG-signed tarballs (no SHA-256), and this host is an
  unsupported Arch/tmpfs environment. Koda therefore keeps reliable discovery
  (`sourcekit-lsp` on `PATH` and the common Swift bin directories), built-in
  highlighting, and a clear capability message rather than shipping an
  unverifiable multi-gigabyte installer. The official URL scheme and
  `gpg --verify` against `https://www.swift.org/keys/all-keys.asc` were
  confirmed and are recorded for a future managed implementation.

### Core web tokenizer and wider formatting

- **JavaScript/TypeScript tokenizer.** Regex and JSX literals are now
  recognised instead of being treated as operators: a `/` in expression position
  (after `=`, `(`, `,`, `return`, …) starts a regex literal, while division after
  a value stays an operator; `<Tag …>`, `</Tag>`, fragments and self-closing
  elements highlight their tag names, attributes and attribute strings. Generics
  (`Array<Foo>`) and comparisons (`a < b`) remain operators. Regression tests
  cover each ambiguous case.
- **CSS depth.** Custom properties (`--brand`) and common functions (`calc`,
  `var`, gradients, transforms, filters, …) are highlighted distinctly, alongside
  the existing selector, property, at-rule and colour support.
- **Formatting across more languages.** The shared formatter pipeline now
  covers Prettier for web, HTML, CSS, JSON, YAML and Markdown; `clang-format`
  for C/C++; `shfmt` for Shell; and `perltidy` for Perl, in addition to
  `rustfmt`/`gofmt`. Every formatter reads stdin and writes stdout, so unsaved
  edits are formatted in place, and each is discovered automatically with an
  accurate availability reason. Prettier and `shfmt` have trusted one-action
  installs (`npm`, `go install`); `clang-format` and `perltidy` ship with their
  toolchains.
- **Assembly server probe fix.** `asm-lsp` rejects `--version`, so it is now
  verified by starting its stdio server (like the HTML/CSS, Kotlin and SQL
  servers). The SQL (`sqls`, via `go install`) and Assembly (`asm-lsp`, via
  `cargo install`) install paths were live-verified end to end, including the
  LSP handshake.
- **Broader toolchain discovery.** Koda now also searches ElixirLS escripts
  (`~/.mix/escripts`), asdf shims, `local::lib` Perl, Swift toolchains
  (`~/.swiftly`, `/usr/local/swift/usr/bin`, `/usr/lib/swift/bin`), a
  Flutter-bundled Dart SDK, `/usr/local/go/bin` and `/snap/bin`, so an existing
  toolchain is found even when it is not on a GUI session's `PATH`. The Dart
  language server is launched with its current `dart language-server` command.

### Kotlin and seven more languages

- **Kotlin.** A built-in, offline provider (comments, regular/raw strings,
  templates, annotations, keywords, modifiers, numbers, operators; structural
  diagnostics; classes, interfaces, objects, functions, properties, type aliases
  and packages as symbols; completion, hover and within-file navigation).
  Detection covers `.kt`/`.kts`; Gradle Kotlin-DSL markers
  (`build.gradle.kts`, `settings.gradle.kts`) are their own project kind while
  the Groovy DSL stays Java. Projects scaffold a Gradle Kotlin build.
- **Kotlin is fully automatic.** `kotlin-language-server`'s bundled compiler
  rejects the four-part version string of Koda's managed JDK 25, so Koda
  provisions a **separate, checksum-verified Eclipse Adoptium JDK 21** under
  `tools/kotlin-jdk` and launches the server with `JAVA_HOME` pointed at it,
  leaving the JDK 25 used by `jdtls` untouched. The pinned server release is
  downloaded, verified by launching it, and its `bin` directory is searched. A
  live test proves the pair completes the LSP handshake.
- **Seven more languages** — Swift, Ruby, SQL, Assembly, Perl, Dart and Elixir —
  each with built-in highlighting, structural diagnostics where meaningful,
  symbols, completion and within-file navigation, plus detection, project
  markers and scaffolding. SQL state/table symbols, Ruby classes/modules/methods,
  Assembly labels, Perl packages/subs, Dart classes/mixins/enums, Elixir
  modules/defs and Swift types/functions are all recognised.
- **Tooling for the new languages** follows the zero-configuration rule:
  `sqls` installs through `go install`, `solargraph` through `gem install` and
  `asm-lsp` through `cargo install` without user setup. Dart, Elixir, Swift and
  Perl servers need a toolchain Koda does not manage, so they are discovered
  when present and reported honestly rather than advertised as installable.
- **Archive extraction fix.** `unzip` has no `--strip-components`, so a stripped
  zip is now unpacked to a scratch directory and its single root moved up — the
  Kotlin server previously landed under `server/`.

### Lua support

- Added **Lua** as a built-in, offline language: `--` line comments,
  `--[[ … ]]` block comments (carried across lines), single- and double-quoted
  strings, `[[ … ]]` and `[=[ … ]=]` long strings, numbers, keywords, builtins
  and operators. Structural diagnostics reuse the shared delimiter checker, and
  functions and methods (`function M:run`, `M.stop = function`) appear in the
  symbol outline with within-file completion, hover, definition and references.
- Detection understands `.lua`, a `lua` shebang and a `.luarc.json` project
  marker (which also gives Lua its own workspace kind). New projects scaffold
  `init.lua` + `.luarc.json`.
- **Zero configuration:** Koda provisions `lua-language-server` itself. Its
  release archive is self-contained (no Node, JDK or other runtime), so Koda
  downloads and unpacks it under its data directory, verifies it with the
  server's own `--version`, and starts it over stdio — giving completion,
  hover, diagnostics, navigation, symbols, rename and code actions with no
  manual setup at all. A live handshake test guards the whole path.

### Soft wrap

- Added **soft wrap** for long lines (`Alt+Z`, or **Toggle Soft Wrap** in the
  command palette). Wrapping is purely visual: no characters are inserted and
  logical positions, selections, diagnostics, multiple cursors and undo/redo are
  unchanged. The character ↔ display-column mapping and the wrap boundaries live
  in `editor/layout.rs`, shared by the renderer and the editor so they can never
  disagree about where a line breaks.
- Up/Down move by visual row and remember the display column, so travelling
  through wrapped and unwrapped lines keeps a stable column. The viewport scrolls
  in visual rows with the same scrolloff, and jumping to a distant position
  recenters on the cursor's line so the work stays bounded to the visible rows.
- Continuation rows keep the gutter blank so one logical line still reads as one
  line number. Inline diagnostic notes appear on a line's last visual row, and
  the completion/hover popups stay anchored to the wrapped cursor.
- Horizontal scrolling is disabled while wrapping and restored when it is turned
  off; wrapping adapts to terminal resizing on the next frame.

### PHP support

- Added **PHP** as a built-in, offline language. A focused scanner highlights
  `<?php … ?>` tags, `//`, `#` and `/* */` comments (carried across lines),
  single- and double-quoted strings, backtick shell strings, `$variables`,
  keywords, constants, builtins, numbers and operators. Structural diagnostics
  reuse the shared delimiter checker, and functions, classes, interfaces,
  traits, enums, constants and namespaces appear in the symbol outline, with
  within-file completion, hover, go-to-definition and find-references.
- Detection understands `.php`, `.phtml` and friends, a `php` shebang, PHP
  content hints and a `composer.json` project marker (which also gives PHP its
  own workspace kind). New projects can be scaffolded with `composer.json` +
  `index.php`.
- `phpactor` is discovered and driven over LSP when it is installed. There is no
  portable user-local installer, so Koda does not provision it; it reports the
  tool honestly and the built-in provider keeps PHP useful without it.

### Multiple cursors

- **Multi-cursor editing.** `Ctrl+D` now adds a cursor at the next whole-word
  occurrence instead of moving a single selection, and `Ctrl+Shift+L` puts a
  cursor on *every* occurrence of the current word or selection. `Ctrl+Alt+↓`
  and `Ctrl+Alt+↑` stack carets on the lines below/above (the same commands are
  in the palette, so they are reachable even where the terminal does not report
  the chord). `Esc` ends the session, and any navigation collapses back to the
  single primary cursor.
- Typing, auto-pairing, newline (with per-line indentation), backspace, forward
  delete, paste, indent and outdent all apply at every cursor. The whole
  keystroke is recorded as **one undo step**, so a single `Ctrl+Z` restores the
  file exactly. Rows are made unique before indentation, so two cursors on one
  line cannot indent it twice.
- Secondary carets render as solid accent blocks, secondary selections share the
  selection background, and the statusline reports the cursor count.

### HTML/CSS tooling and a crash fix

- **The HTML provider no longer crashes on non-ASCII documents.** The tag-balance
  scanner measured positions in characters but sliced each line by *bytes*, so
  ordinary content — `Café`, CJK text, emoji — could land an index mid-codepoint
  and panic the worker that computes diagnostics. The scanner now works entirely
  in characters, with a regression test covering accented text, CJK, emoji and
  comments.
- **HTML/CSS language servers are now recognised after installation.** Koda
  probes a tool by running `--version`, but the extracted VS Code servers reject
  every version query (they require a connection mode), so a successful
  `npm` install was reported as missing and the install was reported as failed.
  These servers are now verified by launching the real server over stdio and
  confirming it stays up, so one install is enough and the language features
  come online.
- **Koda now provisions Node.js and npm itself.** If neither `node` nor `npm` is
  present, installing an npm-based language server first downloads a
  checksum-verified Node.js LTS runtime (`SHASUMS256.txt`, verified before
  extraction) into Koda's data directory, unpacks it, and uses its bundled
  `npm` to install the package into Koda's managed prefix. A working system
  Node.js is still reused when present. Managed Node is placed on `PATH` for the
  install and for launching `#!/usr/bin/env node` servers, and its `bin`
  directory is searched when locating tools.
- **Tool probes are bounded.** A `--version` probe now runs with a timeout and a
  capped output buffer instead of blocking the background worker indefinitely.

### Reliability & security audit

A focused debugging, reliability, security and performance pass. No new
features; the goal was to make existing behaviour trustworthy.

- **Data safety.** Files are now written through a sibling temporary file and
  an atomic rename, so an interrupted or failed save can never truncate the
  original. Loading refuses binary, non-UTF-8 and oversized files instead of
  replacing invalid bytes and writing them back on the next save, and the LSP
  workspace-edit path for files that are not open uses the same atomic writer.
- **File operations.** Rename and copy refuse to overwrite an existing
  destination, so a mistyped name cannot silently destroy another file.
- **Git.** `git status` is parsed from `--porcelain -z`, which keeps leading
  status columns, spaces, non-ASCII names and renames intact (previously the
  first entry could be shifted and misreported as staged). Committing is scoped
  to the workspace root so a subdirectory of a larger repository cannot stage
  unrelated siblings.
- **Editor correctness.** Undoing back to the last save point now clears the
  modified marker; merged backspaces restore the cursor to where the run began;
  closing an earlier tab keeps focus on the same document; and indent, outdent,
  move-line, delete-line and toggle-comment no longer touch a trailing row that
  a selection only reaches at column 0.
- **Detection.** Provider descriptors are ordered deterministically (the
  registry is a hash map), the content-hint cap is exact, and the
  project-context bonus only applies to a language with a file-level signal, so
  a `Cargo.toml` can no longer turn a Markdown file into Rust.
- **Language servers.** Documents opened after a server is already ready are
  now sent `didOpen`; closing a tab (or all tabs, or deleting a path) sends
  `didClose`; a handshake no longer forgives an immediate crash, so the restart
  budget is genuinely bounded; one malformed JSON body no longer tears down a
  live connection; and `documentChanges` is preferred over `changes`.
- **Tool provisioning.** Install subprocesses run from Koda's own tools
  directory, so a repository-local `.npmrc` cannot redirect `npm install -g` to
  an attacker-controlled registry. Downloads and install commands are bounded
  in time and output, the Adoptium JDK install fails closed without a checksum,
  a reclaimed lock can no longer delete its successor, and a strategy is only
  reported installed when its own steps completed and a re-probe succeeds.
- **Adversarial input.** The custom regex and `.gitignore` matchers no longer
  backtrack exponentially (a search-highlight or project-open freeze), and the
  `.gitignore` fix also gives `.gitignore` the correct precedence over
  `.git/info/exclude`.

### Language-server correctness (second audit pass)

- **Position encoding.** Koda now honours the encoding a server negotiates.
  Character positions are converted to and from UTF-8 bytes, UTF-16 code units
  or code points at every boundary, so diagnostics, completion, hover,
  navigation, signature help, rename, formatting and code-action edits are
  correct on lines containing accented characters, CJK, emoji and combining
  marks. Out-of-range or mid-code-point offsets are clamped to a character
  boundary, and signature parameter labels use the UTF-16 offsets the protocol
  requires.
- **Stale responses.** Definition, references, rename, formatting and code
  actions record the document version at request time and are discarded if the
  document changed or closed, or (for navigation) is no longer active. Code
  actions are re-validated when applied.
- **Request lifetimes.** Every request has a deadline (formatting a longer one);
  unanswered requests are expired and their UI state cleared, including on a
  crash or restart, so a hung server cannot leave Koda waiting forever.
- **Diagnostics.** A published document version is parsed and filtered: results
  for an older synchronized version are dropped by the client, and results for a
  buffer whose change has not yet been sent are dropped by the app.
- **Synchronization.** `textDocumentSync` is parsed; a server that advertises
  none is not sent documents. Koda otherwise sends whole-document changes (the
  protocol's safe fallback), including for incremental servers.

### Reliability hardening

- **Background isolation.** A small worker pool runs background requests, so a
  slow formatter, installer or git command no longer blocks detection,
  diagnostics and search. Automatic diagnostics snapshots are dropped when the
  queue is saturated. Formatters and tool commands share a bounded, timed
  subprocess helper.
- **Parsing safety.** JSON-RPC header lines are bounded, numeric-string request
  ids are accepted, and Windows `file:///C:/…` URIs decode correctly.
- **State and search.** Session cursors stay aligned with files when an entry is
  malformed; the recent and session stores are written atomically; project-search
  columns are measured in the original line.
- **Filesystem and scaffolding.** Symlinked directories are expandable (with
  cycle protection in quick open), significant leading whitespace in
  `.gitignore` is preserved, and project names that could break out of generated
  shell, string-literal or HTML contexts are rejected.

### Welcome screen

- The welcome screen now opens on one of five **animated scenes** — *starry
  night*, *cozy desk*, *rainy window*, *sakura drift* and *quiet study* — each
  composed on a small character canvas rather than hand-aligned text. Stars
  twinkle, rain falls over a lit city skyline, petals drift, a lantern glows and
  the Koda familiar cycles through expressions. Cycle scenes with `v` on the
  welcome screen or **Change Welcome Scene** in the palette.
- Added **Toggle Animations** (palette): turning motion off freezes the scene on
  its first frame and stills the busy sparkle, for a calm, reduced-motion
  experience. Motion is on by default.
- Koda now always opens on its **welcome screen** — an interactive home screen
  built around the animated Koda familiar. Move with `↑`/`↓` and open with
  `Enter`; it offers opening a file, opening a project, creating a new project,
  resuming the workspace's saved session, reopening recent projects or files,
  and the shortcuts cheatsheet.
- A path passed on the command line is preserved as an **Open <path>** action
  instead of being opened automatically, and a saved session is offered as
  **Resume <project>** — neither is discarded.
- Recent projects and files persist between launches in the user's state
  directory (`recent.rs`).
- Returning home is a palette command (**Welcome Screen**) and the natural
  result of closing the last tab.
- Session saving is gated on the user having engaged with a project, so
  starting Koda and quitting does not overwrite an untouched session.
- Empty states are now personable and contextual: an empty picker shows a Koda
  a familiar pose, a short line and a hint rather than a bare "no matches", and
  each pose reflects the situation (asleep for a clean working tree, curious for
  a missing search, proud for a tidy codebase). The file-tree filter greets an
  empty result with the familiar too.

### Project creation

- Added a guided **Create New Project** flow: choose a parent folder, name the
  project (validated for the platform and checked for collisions) and pick a
  language. `Esc` steps back at every stage and cancels from the first.
- Scaffolding is deterministic and offline. Koda writes conventional files
  directly rather than invoking `cargo`, `go` or `pip`, so it works with no
  network access and no toolchain: `Cargo.toml` + `src/main.rs`; `go.mod` +
  `main.go`; `pyproject.toml` + a source package; `package.json` +
  `tsconfig.json` + `src/index.ts` (TypeScript) or `package.json` +
  `src/index.js` (JavaScript); `pom.xml` + `src/main/java/…/App.java` (Java);
  a `*.csproj` + `Program.cs` (C#); `CMakeLists.txt` +
  `src/main.c`/`src/main.cpp` (C/C++); or an executable shell script.
- Creation runs on the background worker with a busy indicator and a toast; on
  success the project opens, its entry file loads and language tooling starts
  automatically. If a write fails partway, the partial directory is left
  untouched and Koda reports what happened.
- New palette commands: **Open Project…** and **Create New Project…**, and a
  directory browser used by both. Templates are available for Rust, Go, Python,
  TypeScript, JavaScript, Shell, C and C++.

### License

- Relicensed Koda under the **GNU General Public License v3.0** (previously
  declared MIT). The full text is in [`LICENSE`](LICENSE).

### Zero configuration

- Tool installation is now **user-local, so a permission error can never block
  it**. Koda installs npm-based tools (`bash-language-server`) into a prefix it
  manages under the user's data directory (with a matching user-writable cache)
  instead of the system-wide prefix that `npm install -g` uses by default, and
  only falls back to the user's own global prefix (nvm, fnm, volta, …)
  afterwards.
- Python provisioning no longer assumes `pip` exists. Koda's Python strategy
  installs `python-lsp-server` into a **virtualenv it manages** under its data
  directory first; creating the virtualenv seeds its own `pip`, so a Python
  without the `pip` module — or one marked externally managed (PEP 668) —
  installs cleanly without touching the system environment. `pipx` and `uv` are
  preferred when present, and `ensurepip`/`pip --user` are kept as fallbacks.
- A Rust toolchain with no `rustup` can be bootstrapped from the official
  `https://sh.rustup.rs` installer (`--no-modify-path`, so the user's shell
  profile is left alone), after which the component is added.
- Koda only offers to install a tool when one of its package managers is
  actually present, so it never promises an install it cannot perform. It also
  searches more user bin directories (`~/go/bin`, pnpm, its managed npm prefix
  and Python virtualenv), so a user-local install is found even when those
  directories are not on `PATH`.
- Tool discovery now searches `PATH` and then a handful of well-known user bin
  directories (`~/.cargo/bin`, `~/.local/bin`, `~/bin`, …). Koda is often
  launched from a GUI or a non-login shell whose `PATH` omits exactly the
  directories rustup and pip install into, so this makes automatic server
  startup and formatting work without the user fixing their environment. The
  resolved path is used to launch servers and formatters, not just to probe.
- **Language Setup…** can now install a missing tool with one action, using
  only official acquisition paths: `rustup component add …` for Rust tooling,
  `go install …@latest` for `gopls`, `pipx`/`uv`/a managed virtualenv for
  Python's `python-lsp-server` and `npm` for `bash-language-server`.
  Installation tries each candidate strategy in turn — a machine without
  `pipx`, or with a Python that has no `pip`, still succeeds through the
  managed virtualenv. Koda runs no bespoke package downloader, so provenance
  and integrity remain the package managers' responsibility.
- Every install strategy is verified by **re-probing the tool**, not by
  trusting a package manager's exit code. A tool whose whole toolchain is
  missing (for example no `go` or `npm`) is shown with the missing prerequisite
  instead of a dead install action.
- Tool installation takes an advisory **lock** over Koda's managed tools
  directory, so two Koda instances sharing a data directory cannot install into
  the same npm prefix or Python virtualenv at once. A lock left by a crashed
  instance is reclaimed after a timeout, so an interrupted install never wedges
  provisioning.
- Koda can now **provision runtimes**, not just package-managed tools. Installing
  the Java server fetches a **checksum-verified Eclipse Adoptium JDK** and
  Eclipse JDT into Koda's data directory; installing the C# server fetches the
  **.NET SDK** and a self-contained OmniSharp. Both are launched with their
  runtime on `PATH` (`JAVA_HOME`, `DOTNET_ROOT`) so the user's system is never
  modified. Downloads are HTTPS-only, archive extraction is handled by Koda, and
  every install is still verified by re-probing the tool. Platforms without a
  published runtime are reported honestly rather than half-installed.

- Installation runs on the background worker with a busy indicator; Koda
  re-probes when it finishes and, if a server is now available, starts it.
  Failures (for example, offline) are reported verbatim and editing continues.
- When a Rust, Go or Python file is open and the language server is missing but
  installable, Koda now offers to install it once per session, from the event
  loop so the prompt never blocks startup. Choosing **Not now** keeps the
  built-in intelligence and the offer stays available in **Language Setup…**.

### Reliability

- The project tree and git status now refresh when Koda regains terminal focus,
  so files created or removed by another tool appear without a restart. **F5**
  (or **Refresh File Tree** in the palette) refreshes on demand.
- External-change detection: Koda watches open files for on-disk changes
  (throttled to about once a second). A clean file is reloaded automatically
  with a status message; a file with unsaved edits is preserved and you are
  warned instead of having your work overwritten.

### Session

- Koda remembers the open files, cursor positions and expanded directories for
  each project and restores them on the next launch. State lives in the user's
  state directory (`$XDG_STATE_HOME/koda` or `~/.local/state/koda`), keyed by the
  project root — never inside the project — and is written on quit. Opening a
  file directly (`koda src/main.rs`) still bypasses the saved session.

### Navigation

- Koda now honours `.gitignore`. A dependency-free matcher reads the root
  `.gitignore`, nested `.gitignore` files and `.git/info/exclude`, then hides
  ignored paths from the file tree, quick open, the inline tree filter and
  workspace symbol search. It supports comments, negation, directory-only
  patterns, anchoring and `*`/`?`/`**` wildcards; character classes are not
  supported yet. **Toggle Hidden Files** reveals ignored entries again.

### Language support

- Added **HTML** and **CSS** providers. HTML highlights tags, attributes and
  entities, reports tags that are never closed or mismatch as diagnostics, and
  exposes `id` values as symbols. CSS highlights selectors, properties, values,
  at-rules, hex colours and units, reports unbalanced braces, and exposes
  selectors as symbols. Both work offline for highlighting, diagnostics,
  completion and hover, and gain formatting and richer intelligence from
  `vscode-html-language-server`/`vscode-css-language-server`
  (`vscode-langservers-extracted`), provisioned into Koda's npm prefix.
- Added **Java** and **C#** built-in providers. Java highlights annotations,
  primitive and common library types, strings and text blocks, line/block
  comments, and extracts packages, types, methods and fields as symbols. C#
  highlights attributes, regular/verbatim/interpolated/raw strings and
  XML-doc comments, and extracts namespaces, classes, structs, interfaces,
  records and enums. Both work fully offline for highlighting, structural
  diagnostics, completion, hover and within-file navigation.
- Project detection now understands **suffix markers** (`MyApp.csproj`,
  `App.sln`) in addition to exact names, and recognises Java (`pom.xml`, Gradle
  files) and C# (`global.json`, `*.csproj`/`*.sln`) projects.
- Added deterministic, offline scaffolding for **Java** (a Maven layout:
  `pom.xml` and `src/main/java/…/App.java`), **C#** (a `*.csproj` and
  `Program.cs`) and **HTML** (`index.html` and `style.css`).
- Added **C** and **C++** support. One built-in, offline scanner serves both:
  highlighting for preprocessor lines, line and block comments, strings and
  character literals, numbers, keywords, types and standard-library functions;
  structural diagnostics; symbols (functions, structs, classes, enums, unions,
  typedefs and `#define` constants); completion, hover and within-file
  navigation. Detection understands `.c`/`.h` and `.cc`/`.cpp`/`.cxx`/`.hpp`
  and friends. `clangd` is used when it is already on the system (it ships with
  most C/C++ toolchains); there is no portable user-local installer, so Koda
  reports it as missing rather than pretending to install it.
- Added **TypeScript** and **JavaScript** support. Both share one built-in,
  offline scanner: highlighting for comments, block comments (carried across
  lines), strings, template literals, numbers, keywords, types and decorators;
  structural diagnostics; symbols (functions, classes, interfaces, type aliases,
  enums, constants and namespaces); completion, hover and within-file
  navigation. Detection understands `.ts`, `.tsx`, `.mts`, `.cts` and
  `.js`, `.jsx`, `.mjs`, `.cjs`. `typescript-language-server` is provisioned
  through Koda's user-local npm prefix for rename, code actions and type-aware
  analysis.
- Added **Shell** support (bash, zsh and POSIX sh): built-in highlighting for
  comments, strings, variables and expansions, keywords, builtins and function
  definitions, plus symbols, completion, hover and navigation. A
  `bash-language-server` can be provisioned for fuller analysis.
- Language detection now uses the file's **nearest** project markers rather
  than only the workspace root's. A `.rs` file inside `crates/a/` of a monorepo
  is corroborated by `crates/a/Cargo.toml` even when the workspace root declares
  no project, so confidence is accurate away from the root.
- Added **Python** support, entirely through Koda's built-in intelligence so it
  works offline with nothing to install: syntax highlighting (triple-quoted
  strings, decorators, f-string text), structural diagnostics, symbols
  (functions, classes, module constants), completion, hover and within-file
  navigation. Rename and code actions remain unavailable until a Python
  language server is offered.
- Detection now corroborates the file's own language when a repository has
  several project markers, so a `.rs` file next to a `pyproject.toml` is still
  confidently Rust (and vice versa) instead of depending on descriptor order.
- Added built-in support for **Markdown, JSON, TOML and YAML**. Each gets
  syntax highlighting with no setup, so documentation and configuration files
  are no longer plain text.
- JSON, TOML and YAML reuse the shared delimiter checker for structural
  diagnostics (unbalanced braces, brackets and quotes), with JSONC `//` and
  `/* */` comments and TOML multi-line strings understood. Markdown skips
  diagnostics because brackets are ordinary prose.
- Symbol outlines now cover prose and config: Markdown headings, JSON top-level
  keys, TOML tables and root keys, and YAML top-level keys appear in
  **Go to Symbol** (`Ctrl+Shift+O`) and **Workspace Symbols** (`Ctrl+T`).
- Completion offers each data format's literals, and `#`/`//` are wired up for
  **Toggle Comment**.
- `Cargo.toml` is detected as TOML (a file-name signal) while still marking a
  Rust project; the TOML provider deliberately does **not** claim `Cargo.toml`
  as a project marker, so Rust project context is unaffected.

### Language servers

- **One server per language.** Koda now keeps a language server for each
  language in the workspace instead of only the first one it detected, so a
  project that mixes Rust and Python (or a split pane showing two languages)
  gets tooling for both. Each server has its own handshake, restart budget and
  failure state; the statusline shows a connection when any server is ready,
  and workspace-wide requests prefer the active document's server. Servers are
  started independently and a failure in one never disturbs another.
- **Protocol hygiene.** Koda asks a server for `utf-8` character offsets and
  uses them when offered, so positions stay correct in files that contain
  non-ASCII text. It only sends a feature request when the server advertised
  that capability (`completionProvider`, `hoverProvider`, …), so built-in
  intelligence takes over cleanly instead of showing a spurious error. A server
  that crashes now reports its last stderr lines alongside the failure.
- **Signature help.** Typing `(` or `,` in a call asks the server for parameter
  hints and shows them in a small popup anchored below the cursor, with the
  active parameter emphasised, the signature's documentation and a counter when
  overloads exist. It is gated on the server's `signatureHelpProvider`
  capability, refreshes as the arguments change, and is dismissed by leaving
  the call or pressing any non-typing key. A superseded response is discarded by
  request id.
- **Formatting over LSP.** When a server advertises `documentFormattingProvider`
  (Eclipse JDT, OmniSharp, `typescript-language-server`, …) **Format Document**
  sends `textDocument/formatting` and applies the returned edits as one undoable
  change, using the file's detected indentation width. The built-in
  `rustfmt`/`gofmt` path remains for languages without a server.
- Added an asynchronous language-server client. When a supported server is
  installed (`rust-analyzer` for Rust, `gopls` for Go) Koda starts it for the
  workspace, runs the LSP handshake, keeps documents in sync and shows the
  server's `publishDiagnostics` through the existing diagnostics UI — gutter
  markers, underlines, the statusline count and `F8` navigation.
- Servers start lazily (~600 ms after a file is opened) so opening files is
  instant, and if one exits Koda hands diagnostics back to its built-in
  providers. With no server installed Koda keeps its heuristics, so offline
  editing is unaffected. Completion, hover, navigation, rename and code actions
  over LSP come next.
- Language-server **completion, hover, go-to-definition and references** now
  flow through the connection. Completion merges the server's candidates with
  the instant local ones; hover upgrades the built-in popup when the server
  answers; definition and references jump across files. The UI is identical
  whether an answer came from a server or a built-in provider.
- **Rename** (`F2`) asks the server for a workspace edit and applies it across
  every affected file — open buffers as undoable edits, unopened files on disk.
  The palette reports "needs a language server" when none is attached.
- **Code actions** (`Ctrl+.`) list the server's quick fixes and refactors in a
  picker and apply the chosen one, whether it carries an edit or a command
  (including server-initiated `workspace/applyEdit`).
- **Workspace symbols** (`Ctrl+T`) now also come from the server's
  `workspace/symbol` request when one is attached, merged with Koda's built-in
  project scan so results appear instantly and remain available offline.
- **Reliability.** A server that never finishes its handshake is abandoned
  after a timeout, and one that exits unexpectedly is retried automatically a
  bounded number of times before Koda settles on its built-in intelligence.
  **Restart Language Server** in the palette reconnects on demand. Every
  failure falls back cleanly, so editing never depends on a server being up.

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

- Completion is now offered **automatically while you type**. After a short
  pause (about 120 ms) Koda opens the popup at the cursor and re-filters it as
  the word grows; it stays quiet inside comments and strings, dismisses on
  whitespace or punctuation, and does not pop when the only candidate is the
  word already being typed. `Ctrl+Space` remains the manual fallback and shows
  the full list. Language-server candidates are requested for the same prefix,
  and a response superseded by a newer request is discarded, so a stale answer
  never overwrites the current list. After `.` or `::`, buffer words are left
  out so member completion is not buried in noise.
- Completion matching is now fuzzy with a prefix bias: `mrs` finds
  `main_result`, while an exact prefix (`if` over `impl`) still ranks first.
  This applies to both built-in and language-server candidates.
- **Go to definition now works across files without a language server.** When
  the word under the cursor is not defined in the current file, `F12` searches
  the project for a same-named symbol and opens the workspace-symbol picker
  prefilled with that word, so offline navigation no longer stops at the file
  boundary.
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
- Added language-tool discovery. Koda probes for `rust-analyzer`, `gopls`,
  `rustfmt` and `gofmt`, checking that they exist *and* run (a `rustup` shim can
  exist for a component that is not installed). **Language Setup…** lists every
  tool with its version or an install hint. Automatic provisioning is not
  implemented yet.

### Performance

- Added a background worker thread. Language detection and git status now run
  off the UI thread and their results are applied as they arrive, so opening a
  file, saving, or starting Koda never waits on `git status` or disk reads.
  This is the foundation for asynchronous language intelligence (diagnostics,
  completion, and so on).
- Koda repaints only when input arrives, background work completes, a status
  message expires, or the terminal is resized.

### Polish & motion

- Background results now appear as short-lived **notifications** stacked above
  the statusline: tool installs, commits, formatting, git errors and language
  server recovery each show a severity-coloured toast. Consecutive duplicates
  are suppressed and the most recent few are kept.

- Diagnostic messages can now appear inline at the end of the affected line
  (an error-lens style note), coloured by severity, truncated to fit and
  suppressed on narrow or horizontally scrolled lines. **Toggle Inline
  Diagnostics** in the command palette turns them on or off.

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

### Git

- Added **Changed Files…** (`Ctrl+Shift+G`): a filterable list of every file
  with a working-tree status (modified, added, deleted, renamed, untracked,
  conflicted), each showing its short indicator and whether it is staged.
  Choosing one opens it; pressing `Space` stages or unstages the selected file,
  and the list refreshes from the new snapshot. A **Stage / Unstage File**
  command does the same for the tree selection or active file.
- Added **Commit Changes…**: prompts for a message, stages every change
  (`git add -A`) and commits on the background worker, then refreshes the
  status bar and changed-files list. Failures — such as a missing git identity
  — are reported verbatim. Koda never rewrites history; the action is explicit.
- Added a **diff view**. **Diff File** in the palette shows the active file's
  unified diff in a scrollable, colour-coded panel (added lines green, removed
  red, hunks as headers); the working tree is preferred and the staged diff is
  shown when the working tree is clean. In the changed-files list, `d` opens
  the selected file's diff and the footer advertises the key. Untracked files
  are shown as entirely new.

### Search

- **Regex search.** Press **Alt+R** in the find bar to interpret the query as
  a regular expression. Koda ships a small, dependency-free engine supporting
  literals, `.`, `*`/`+`/`?`, classes, `^`/`$` and `\d`/`\w`/`\s`; unsupported
  syntax (groups, alternation, `{n,m}`) is reported inline rather than matching
  the wrong thing.

- Added **Search in Project…** (`Ctrl+Shift+F`): a case-insensitive text
  search across every file Koda knows about, run on the background worker and
  shown as a filterable list of matches that jumps to the chosen line. It
  respects `.gitignore`, skips binary and oversized files, and reports at most
  one match per line. A single-line selection prefills the query.

### View

- Added a **split editor** (`Alt+V`): two documents render side by side with a
  hairline rule, and `Alt+O` moves editing focus between the panes. The focused
  pane owns the cursor, the active tab and search highlighting; tab switching
  applies to the focused pane. Closing a split document collapses the view
  cleanly, and the split is skipped on terminals too narrow for two panes.

### Keymap

- `Ctrl+B` now moves focus into the file panel (revealing it if hidden) and
  hides it on the next press, so the sidebar is reachable without a mouse;
  `Ctrl+E` still toggles focus without hiding.
- Koda asks the terminal for distinct modified keys (the kitty keyboard
  protocol) and treats an uppercase character as shifted, so `Ctrl+Shift+P`
  opens the command palette rather than quick open on terminals that would
  otherwise report it as a plain `Ctrl+P`.
- Added `Alt+D` **Diff File**, `Alt+I` **Toggle Inline Diagnostics** and `v`
  (on the welcome screen) to cycle the scene. A keymap test now asserts that
  every command id and every shortcut is unique, so future bindings cannot
  silently collide.
- Rebound **Go to Matching Bracket** from `Ctrl+M` to `Alt+M`: most terminals
  encode `Ctrl+M` as Enter, so the old binding was effectively unreachable
  (it still works on terminals with enhanced keyboard reporting).
- Added `Ctrl+Shift+S` **Save As**, `Ctrl+N` **New File**, `F3`/`Shift+F3` for
  **find next / previous**, `Ctrl+PageUp`/`Ctrl+PageDown` for **tab switching**
  and `Ctrl+Shift+M` for the **diagnostics list**.
- `Ctrl+Shift+S` previously fell through to a plain Save.

### Editing & UX

- Added **Select Next Occurrence** (`Ctrl+D`): with no selection it selects the
  word under the cursor, and each further press selects the next whole-word
  occurrence in the file, wrapping around at the end. It is the single-cursor
  basis for multi-cursor editing.
- Inferred **indentation**. On opening a file Koda detects its indentation unit
  from the leading whitespace — two spaces in a JavaScript file, four in a Rust
  file — and uses it for auto-indent, `Tab`, and indent/outdent, so Koda matches
  the project without any configuration. Indent guides are drawn at the same
  detected width. A file with no discernible style (or a tab-indented one) keeps
  the four-space default.
- Added **Delete Line** (`Ctrl+Shift+K`), which removes every line the cursor
  or selection touches, and **Go to Matching Bracket** (`Alt+M`), which jumps
  between a bracket and its partner using the same nesting-aware scan that
  drives matching-bracket highlighting.
- **Replace All** (`Alt+Enter` in the replace bar, or the palette) replaces
  every match in a single undoable edit, honouring the case, whole-word and
  regex options. One `Ctrl+Z` restores the file.
- Copy and cut now feed a **kill-ring**; **Alt+Y** yank-pops the last paste to
  an earlier kill, as long as nothing has been edited since. The system
  clipboard (OSC 52) still receives the newest kill.
- Added **New File…**, **Rename…** and **Delete…** to the palette. New files
  are created in the selected folder (or the active file's folder) and opened
  immediately. Renaming a file or folder updates any open buffers and the
  recent list; deleting closes the affected tabs and refuses while a file under
  the target has unsaved changes.
- Added **Duplicate File** (a `name copy.ext` sibling, opened immediately) and
  **Copy File…** (copy the selected file to a chosen path).
- Find is case-insensitive by default and gains two toggles while the find bar
  is open: **Alt+C** for case sensitivity and **Alt+W** for whole-word matching.
  The active options are shown in the bar.
- Added **Revert File** to the command palette: it discards local edits and
  reloads the active file from disk.

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
