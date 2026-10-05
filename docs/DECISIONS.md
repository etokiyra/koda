# Design decisions

The *why* behind Koda's implementation. `AGENTS.md` states the rules; this file
keeps the rationale that would otherwise bloat it. Read the section for the
subsystem you are about to touch — for example, read
[Provisioning](#provisioning) before changing `language/tools.rs`, and
[LSP](#lsp) before changing `language/lsp/`.

If a decision here is reversed, say so in [CHANGELOG.md](../CHANGELOG.md), update
this file, and add or change a test.

---

## Editor

- **Rope-backed buffers** (`ropey`). Efficient edits and slicing on large files.
- **Operation-based undo.** Each edit records `(start, removed, inserted,
  cursor_before, cursor_after)`. Cheap and exact.
- **Editor history tracks a save point.** Each edit has a unique id and the
  document remembers the id at its save point, so undo/redo restore the clean
  state exactly; coalescing is broken when a document is saved.
- **Inferred indentation.** Each document detects its indentation unit from the
  file's leading whitespace (falling back to four), and uses it for
  auto-indent, `Tab` and indent/outdent. This is zero-configuration and keeps
  the editor core language-agnostic: the width is a property of the file, not a
  per-language branch.
- **Multiple cursors are a small, explicit model.** The primary stays on
  `Document` as `cursor`/`selection`; `Document::cursors` holds the secondary
  `Cursor`s (anchor + cursor), kept sorted, unique and distinct from the
  primary. Edits are planned per cursor against the pre-edit text and applied
  bottom-up, then recorded as one `Edit` whose `ops` list makes the whole
  keystroke a single undo step. `apply_multi_edit` is the only path that mutates
  several ranges, and it moves cursors even for a zero-op keystroke so bracket
  skip-over works everywhere. Navigation deliberately collapses to the primary;
  every other single-point edit clears the extras so a stale cursor can never
  survive an unrelated operation.
- **Soft wrap is a display-only transform with one source of truth.** A position
  is always a character position; wrapping only adds a derived *visual row*.
  `editor/layout.rs` owns both the character ↔ display-column mapping (tabs
  expand there) and the wrap boundaries, and the renderer and the editor's
  Up/Down movement both call it, so they cannot disagree. The viewport is
  `scroll_top` (a logical row) plus `scroll_subline` (which visual row of it is
  on screen); the cursor's logical line is always kept at or above the top, which
  bounds scrolling work to the visible rows instead of scanning the document.
  Wrapping never edits the buffer, so undo/redo, diagnostics, LSP positions and
  multiple cursors are unaffected; turning it off restores horizontal scrolling.
- **Saves are atomic and lossless.** Writes go to a sibling temporary file that
  is flushed and renamed over the original, preserving permissions. Loading
  refuses binary, non-UTF-8 and oversized files rather than replacing bytes with
  U+FFFD and writing them back, and filesystem rename/copy never overwrite an
  existing destination.
- **Split editor.** The editor keeps a single active document; a split stores
  one document index per pane and `editor.active` follows the focused pane, so
  every existing editing path keeps working unchanged. Panes share the tab
  strip, and closing a pane's file collapses the split.

---

## Language and detection

- **Line-based highlight cache.** Providers return `(spans, next_state)` so
  multi-line constructs work. `HighlightState` carries a block-comment flag plus a
  small shared vocabulary — `LexMode` (template literals and their `${}`
  interpolation) and `Embed` (an open HTML `<script>`/`<style>` region) — rather
  than language-specific state on the editor. The HTML tokenizer delegates an
  embedded body to the JavaScript or CSS tokenizer, which carries its own state,
  so one line can be split between two languages. The cache is invalidated from
  the edited line down.
- **Detection is deterministic and file-first.** Provider descriptors are sorted
  by language id before the engine is built, the content-hint weight is capped
  at exactly three, and the project-context bonus applies only to a language
  with a file-level signal (name, shebang or extension) — a project marker never
  overrides a file's own nature.
- **The web tokenizer uses lookback heuristics, not a parser.** A `/` in
  expression position starts a regex literal (division after a value does not),
  and `<` opens a JSX tag only after an expression start, so `Array<Foo>` and
  `a < b` stay operators. This keeps the built-in highlighter predictable on
  incomplete and ambiguous code; it is a lexical aid, not a semantic parser.
- **Formatting reuses trusted tools through one pipeline.** A provider declares
  the `Formatting` capability and implements `format()` by calling a shared
  helper in `language/format.rs` (`rustfmt`, `gofmt`, `prettier`,
  `clang-format`, `shfmt`, `perltidy`, plus the managed `dart format` and
  `mix format`). Every helper feeds a buffer snapshot on stdin and takes stdout,
  so unsaved edits are formatted in place and the result applies as one
  undoable edit. The provider's `formatter()` names the executable, and
  `Tool::for_language(language, Formatter)` lets the palette explain a missing
  formatter before the user invokes it. Language-server formatting is preferred
  when the server advertises it.
- **A language may have more than one candidate server.** `Tool::ALL` order is
  the preference order and `Tool::available_server` / `Tool::installable_server`
  choose the first server that is actually available or installable. Perl is the
  first user: **PLS** (no `Coro` dependency, so it builds on current Perls) is
  preferred, with `Perl::LanguageServer` discovered as a fallback. The editor
  still tracks one running server per language.

---

## LSP

- **One language server per language.** The app tracks servers in a map keyed by
  `LanguageId`; each has its own handshake, restart budget and failure state.
  Feature requests resolve against the active document's language, and a
  language's server failing never disturbs another. Workspace-wide requests
  prefer the active language's server, then any ready one.
- **Language servers are attached for the whole session.** A document detected
  after the handshake is sent `didOpen`, closing sends `didClose`, the restart
  budget is only forgiven after a stable uptime (so a crash-after-initialize
  loop is bounded), and one malformed message does not tear down a live
  connection. Results are still matched to their request id and document.
- **The LSP encoding boundary is centralized.** Koda's internal positions count
  Unicode scalar values. Everything crossing the LSP boundary is converted in
  `language/lsp/convert.rs` (`char_to_lsp`/`lsp_to_char`, and `edits_to_chars`)
  using the actual line text and the negotiated `PositionEncoding` (UTF-8,
  UTF-16 or UTF-32). Outbound requests convert the cursor; inbound diagnostics,
  edits, formatting and locations convert back before use. Conversion always
  clamps to a character boundary and never splits a code point. Do not add
  ad-hoc character/byte arithmetic in the UI.
- **Server responses are validated against the document they describe.** A
  document-sensitive request records `(language, id, path, buffer version)` at
  request time; a response is applied only if it is still the newest request and
  the document is open at the same version (navigation also requires it to be
  active). Code actions are re-validated when applied. Never apply an edit or
  move the cursor from a response that has been superseded.
- **Requests and subprocesses are bounded.** Every LSP request has a deadline
  (formatting a longer one); expired requests are removed and their UI state
  cleared. External processes run through `process::wait_captured`, which drains
  both pipes, caps output and kills the child at a deadline. New subprocess call
  sites must use it rather than `Command::output` or `wait_with_output`.
- **Background work is isolated.** A small worker pool runs background requests,
  so one slow operation cannot stall unrelated services. Automatic,
  superseding snapshots (diagnostics) may be dropped when the queue is
  saturated; user-initiated work must not be dropped.
- **Adversarial input is bounded.** Search highlighting runs on the UI thread,
  so the regex engine carries a per-line step budget and the `.gitignore` glob
  matcher is polynomial; a crafted pattern degrades to "no match" instead of
  freezing the editor.

---

## Provisioning

- **User-local tool installs.** Provisioning never writes to a system-owned
  location: `rustup`/`go install` place tools in the user's home, Python uses a
  virtualenv Koda manages under its data directory (which seeds its own `pip`,
  so a Python without `pip` or one marked externally managed still works), with
  `--user`/`pipx`/`uv` as fallbacks, and npm-based tools use a prefix Koda
  manages under its data directory (with a matching cache). A missing
  permission therefore cannot make an install fail, and every resulting bin
  directory is searched when locating tools, alongside the usual user bin
  directories.
- **Verified install attempts.** A tool's provisioning is an ordered list of
  self-contained steps — run a command, download an archive (optionally
  verifying a published SHA-256), fetch and verify an Adoptium JDK, or extract.
  Extraction lists the archive first and refuses absolute or `..` paths, failing
  closed if it cannot list it, and unpacks a zip with `unzip`, `bsdtar` or a
  Python interpreter so it does not depend on one distribution's tooling. Koda
  re-probes the tool after every attempt rather than trusting a package
  manager's exit code, and reports a missing prerequisite (a whole absent
  toolchain) instead of offering an install that cannot run.
  Installs into the shared managed directory are guarded by an advisory lock
  (with stale-lock recovery) so concurrent Koda instances cannot corrupt the same
  npm prefix or virtualenv.
- **Managed toolchains.** Some language servers need a whole toolchain, so Koda
  provisions one under its own data directory and launches the server with it:
  the .NET SDK for OmniSharp (`DOTNET_ROOT`), a checksum-verified Eclipse Adoptium
  JDK for Eclipse JDT (`JAVA_HOME`), a dedicated JDK 21 for Kotlin (kept separate
  from JDT's JDK 25), a checksum-verified Dart SDK for `dart language-server`, a
  coordinated Erlang/OTP + Elixir + ElixirLS stack for Elixir, and the official
  Swift toolchain for `sourcekit-lsp`. `Tool::launch_env` supplies the environment
  for both probing and launching; the user's system environment is never modified.
- **Every managed download is verified and pinned.** A `Download` step carries a
  required SHA-256 (there is no way to construct one without a digest), and the
  other download steps verify upstream data: `GithubRelease` (the SHA-256 digest
  GitHub reports for a release asset), `BobBuild` (the checksum `builds.hex.pm`
  publishes), `AdoptiumJdk` (the checksum in the Adoptium API response),
  `NodeRuntime` (`SHASUMS256.txt`), `DartSdk` (the sibling `.sha256sum`),
  `GoToolchain` (the `go.dev` release JSON), `DotnetSdk` (the SHA-512 in
  Microsoft's `releases.json`), `RustupInit` (the `.sha256` beside the versioned
  `rustup-init` binary), `DownloadGpg` (Swift's detached signature, verified
  against a private keyring) and the pinned `App::cpanminus`. Every input is an
  exact version: `jdtls` is a milestone snapshot with its published `.sha256`
  rather than `-latest`; `lua-language-server`, `kotlin-language-server` and
  OmniSharp use a pinned tag with a GitHub digest or a pinned SHA-256; the Go
  toolchain and the Adoptium JDKs are pinned by version; and package-manager
  installs (`go`, npm, pip, gem, cpan, `cargo`) name an exact version, leaving
  only `rustup component add` and `mix local.hex`/`local.rebar` as delegations
  to the manager (tracked in [LIMITATIONS.md](LIMITATIONS.md#provisioning)).
  Downloads use a stall-detecting, long-transfer timeout and are refused when
  the target filesystem is too small (the download-size estimate, or an
  archive's own size before extraction). HTTP(S) proxy variables are forwarded
  to `curl`. Each install is bounded, locked and re-probed by launching the
  **real server**, and a verified download that still cannot run (a missing
  shared library, a broken launcher) reports the real reason instead of a
  generic failure. Hashing goes through whichever tool the system has —
  `sha256sum`/`sha512sum`, `shasum`, `openssl dgst` or a Python interpreter —
  and a value that is not the right length of hex is rejected, so a malformed
  tool output cannot pass verification.
- **Platform support is resolved, never guessed.** `swift_platform` and
  `bob_platform` map the running distribution from `/etc/os-release` (including
  `ID_LIKE` derivatives) to the exact upstream artifact. A distribution upstream
  does not build for is reported as unsupported — Koda does not download a
  toolchain that cannot run there — while a glibc system with no explicit `bob`
  build uses a best-effort target and still verifies the result. Perl bootstraps a
  checksum-verified `cpanm` so an interactive, unconfigured `cpan` is never run,
  and installs into an isolated `local::lib` (`PERL5LIB`).
- **Swift is portable, not allowlisted.** A distribution swift.org does not build
  for gets the portable **UBI10** toolchain plus a managed compatibility layer:
  `libncurses.so.6` is aliased to the system's `libncursesw.so.6` (the same
  library, a different soname) and `libxml2.so.2` is linked from the system, both
  from a directory under Koda's data directory and used through a scoped
  `LD_LIBRARY_PATH`. The system is never modified and no library is bundled; if
  the distribution cannot provide `libxml2.so.2`, Koda names the package instead
  of downloading a toolchain that cannot run. `libc_is_musl` keeps the glibc
  build off musl systems.
- **A managed toolchain can provision the tool that installs another tool.** The
  official Go toolchain is one managed component that unlocks `gopls`, `sqls` and
  `shfmt`: `go_attempts` first tries the user's `go`, then a managed Go archive,
  and every `go install` writes to a Koda-private `GOPATH`/`GOBIN`, so nothing
  lands in `~/go`. `gofmt` has no `go install` step — it ships with Go — so its
  plan is the toolchain archive alone. The same "try the ambient tool, then
  provision a self-contained one" shape covers `cargo`/`rustup` for `asm-lsp`.
  Downloads verify the checksum the source actually publishes (`go.dev/dl` JSON,
  GitHub asset digests) and fail closed.
- **Prefer a small, self-contained upstream bundle over a system toolchain.**
  `clangd` comes from the clangd project's own release (a ~120 MB bundle with its
  resource headers) rather than requiring LLVM; `asm-lsp` prefers its prebuilt
  release; `phpactor.phar` removes the Composer dependency. These bundles are
  gated behind `libc_is_musl` where they link glibc. Native dependencies are
  checked, not assumed: only a build whose libraries are present is offered, and
  a missing one is named.
- **A runtime Koda cannot verify is a prerequisite, not a download.** PHP and
  Ruby interpreters are discovered and reused, never fetched: official PHP is
  source-only and the third-party static builds publish no checksum, and Ruby has
  no official portable binary. Koda still isolates what it can — `solargraph`
  installs into a private gem home with `GEM_HOME`/`GEM_PATH` scoped to the
  process — and reports the missing runtime precisely.
- **Provisioning is isolated and bounded.** Install subprocesses run from Koda's
  own tools directory (never the user's project, so a local `.npmrc` cannot
  hijack `npm`), downloads and commands have time and output limits, checksum
  verification fails closed, and the install lock records a nonce so a reclaimed
  holder cannot delete a successor's lock.
- **Downloads pass through a verified, digest-keyed cache.** A download is
  written to a temporary sibling file, verified, and only then renamed into
  place; a verified copy is stored under `<tools>/cache/<algorithm>-<digest>`
  and reused without the network. Cache identity is the expected digest, never
  the filename, and a cached file is re-verified before use — a corrupt entry is
  discarded and redownloaded, and a failed download or checksum mismatch never
  enters the cache. Hard links keep the cache from duplicating large archives.
- **Installation is atomic per step.** An extraction stages into a sibling
  directory and promotes it with a same-filesystem rename, renaming the previous
  installation aside first and restoring it if promotion fails. This is per
  `Extract` step, not a whole-plan transaction: a plan that installs A then
  fails on B may leave A installed, and Koda reports the incomplete plan rather
  than a false success.
- **Install plans are pre-flighted.** `install()` keeps only strategies whose
  every step is available and validates the tools directory and disk space
  before any download, so an obviously impossible plan is refused before work
  starts. Network and archive contents stay runtime checks.
- **Package-manager delegations are explicit.** Hex is pinned to an exact
  version; rebar3 is a checksum-verified download registered through
  `mix local.rebar rebar3 <path>`. Only `rustup component add` remains a
  delegation to an upstream manager's own verification (rustup's signed
  manifests), and it is documented as such in
  [LIMITATIONS.md](LIMITATIONS.md#provisioning).

---

## Project, search and git

- **Deterministic project scaffolding.** `project/create.rs` writes conventional
  project files directly instead of invoking `cargo`, `go` or `pip`, so
  creation is instant, offline and toolchain-independent. It never deletes a
  partial project: a failure returns the root so the UI can explain what
  happened.
- **Lazy, `.gitignore`-aware file tree.** Directory listings are read only when
  expanded; ignored directories (`target`, `node_modules`, …) are skipped, and
  the project's `.gitignore` rules (including nested files and
  `.git/info/exclude`) hide generated and local files from the tree, quick open,
  the inline filter and workspace symbol search. Toggling hidden files reveals
  them again.
- **Git via subprocess.** We shell out to `git` instead of linking a heavy
  library. Failures degrade gracefully (no repo → no git UI).
- **Git is parsed losslessly.** `git status` is read as `--porcelain -z` raw
  bytes, so leading status columns, spaces, non-ASCII names and renames survive;
  staging is scoped to the workspace root.
- **Adversarial input is bounded** (shared with search). The regex engine
  carries a per-line step budget and the `.gitignore` glob matcher is
  polynomial; a crafted pattern degrades to "no match" instead of freezing the
  editor.

---

## UI

- **Centralised Mellow theme + art.** All colour flows through `ui/theme.rs`;
  all ASCII personality through `ui/art.rs`. Widgets render semantic roles, not
  literal colours. This keeps Koda recognisable and easy to evolve.
- **Scenes are drawn on a canvas.** `ui/art.rs` composes each welcome scene by
  placing glyphs at coordinates on a small `Canvas`, then turning runs of equal
  style into spans. This keeps the art symmetric and lets one element animate
  without the rest drifting; every scene is padded to an equal width so
  per-line centering stays aligned. The welcome composer budgets its height and
  drops the scene for just the familiar, or nothing, on small terminals.
- **Motion is a toggle, not a config file.** `App::motion` (palette:
  **Toggle Animations**) freezes the scene and the busy sparkle. Zero
  configuration is preserved; reduced motion is one keypress away.
- **Overlays are centered and cleared** with `Clear` before drawing so they are
  readable over the editor.
- **Transient status messages** expire so the language/git summary returns.
- **Welcome-first startup.** Koda opens on the welcome home screen. A path from
  the command line is offered as an action, a saved session as a resume action,
  and recent projects/files are loaded from the state directory; nothing is
  opened until the user chooses. Session saving is gated on the user engaging
  with a project, so an untouched session is never overwritten.
- **OSC 52 clipboard.** Copy works over SSH and in most terminals; internal
  clipboard is the fallback and bracketed paste handles system paste.

---

## Platform

- **Linux and macOS are supported; Windows is not yet supported.** There is no
  Windows CI, several installers require a Unix `sh`, and path/launcher
  conventions are not handled. See [LIMITATIONS.md](LIMITATIONS.md#platform).

---

## Product and configuration

- **Zero configuration by default; customisation is in-app.** Koda must work
  with no file to write. When settings arrive they are reached from inside Koda
  (a **Settings** screen in the palette); any on-disk settings store under
  Koda's data directory is an implementation detail the user never has to
  maintain. A missing settings file must change nothing. This is a deliberate
  boundary against becoming a config-file-first tool.
- **"Supported" means complete, not "an LSP starts".** A language is supported
  only when detection, offline built-in features, provisioning, server launch,
  server-backed features, lifecycle (version/update/remove) and recovery all
  hold. The contract is written down in
  [PRODUCT.md](PRODUCT.md#what-supported-means); the current per-language state
  is in [LIMITATIONS.md](LIMITATIONS.md).
- **The full product direction lives in [PRODUCT.md](PRODUCT.md).** It is the
  single authoritative statement; `ROADMAP.md` turns it into work and
  `LIMITATIONS.md` records the honest current state.

---

## Splitting `src/app/mod.rs`

`src/app/mod.rs` was 8876 lines of state, an `impl App` and its tests. It has
been split into sibling modules under `src/app/` by **mechanical extraction
only** — one extraction per commit, no behaviour change, the full suite green
after every commit. A child module of `app` sees `App`'s private fields, so each
module holds its own `impl App` block; extracted methods are `pub(super)` so the
event loop and sibling modules can call them.

The originally proposed module set was realised (tests, git, lsp, diagnostics,
session, files, keys, language, commands). Because the proposal's own estimates
for `keys.rs`/`language.rs` exceeded the 800-line rule and it omitted the editing,
pane, search, status and background-event code, the production code was
distributed a little further so every production file is under 800 lines:

| Module | Contents | Lines |
| --- | --- | --- |
| `mod.rs` | state types, constructor, event loop, small helpers | 775 |
| `background.rs` | worker-event application, external-change polling, pumping | 295 |
| `commands.rs` | command dispatch, palette, language setup, prompts, quick open | 708 |
| `completion.rs` | completion popup and hover | 360 |
| `diagnostics.rs` | diagnostics polling, navigation and list | 187 |
| `editor.rs` | cursor moves, clipboard/kill-ring, comment, matching bracket | 222 |
| `files.rs` | save/close/revert and file create/rename/copy/delete | 356 |
| `git.rs` | diff, changed files, staging, commit | 171 |
| `keys.rs` | key dispatch (global, editor, tree, overlay, search, completion) | 763 |
| `language.rs` | navigation, symbols, rename, code actions, formatting | 618 |
| `lsp.rs` | server lifecycle and response handling | 769 |
| `panes.rs` | split panes and tabs | 170 |
| `search.rs` | find/replace | 202 |
| `session.rs` | session persistence, workspace and welcome actions | 396 |
| `status.rs` | statusline messages and toasts | 103 |
| `view.rs` | tree/diagnostic/wrap toggles and reveal | 79 |

`app/tests.rs` (2901 lines) is the deliberately single test module moved whole
from `mod.rs`; the production modules are all under 800. `app/overlay.rs`
(833 lines) predates this work and is not part of the split.
