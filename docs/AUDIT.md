# Koda project health audit

Phase 0 of the project health pass. This document is a **read-only audit**: it
records what the code actually does, with `file:line` evidence, and marks every
claim `true`, `false` or `unverified`. Nothing in `src/` was changed to produce
it.

- **Repository:** `/home/etokiyra/koda`
- **Branch / commit:** `master` @ `b4845af` ("small readme fix")
- **Toolchain:** stable, Rust edition 2024 (`Cargo.toml:4`)
- **Date:** 2026-10-05

Method: ran the three gates, then read the relevant modules. Where a claim could
not be confirmed from code or a test, it is marked **unverified** rather than
assumed. The function-length measurement is a tokenizer-aware brace scan (so
braces inside strings/comments are ignored) that skips the trailing
`#[cfg(test)]` module; it is an approximation, not a parser.

---

## 1. Verification gates

| Gate | Command | Result | Evidence |
| --- | --- | --- | --- |
| Formatting | `cargo fmt --check` | clean (exit 0) | no diff |
| Lints | `cargo clippy --all-targets` | clean, no warnings | finished OK in 20.11s |
| Tests | `cargo test` | **539 passed, 0 failed, 22 ignored, 0 doctests** | see breakdown below |

Real test breakdown (from the actual run):

| Target | Passed | Ignored | Failed |
| --- | --- | --- | --- |
| `unittests src/lib.rs` | 499 | 0 | 0 |
| `unittests src/main.rs` | 0 | 0 | 0 |
| `tests/html_css_tooling.rs` | 0 | 3 | 0 |
| `tests/kotlin_tooling.rs` | 0 | 1 | 0 |
| `tests/lua_tooling.rs` | 0 | 1 | 0 |
| `tests/optional_servers.rs` | 0 | 17 | 0 |
| `tests/provider_robustness.rs` | 1 | 0 | 0 |
| `tests/render.rs` | 39 | 0 | 0 |
| doctests | 0 | 0 | 0 |
| **Total** | **539** | **22** | **0** |

The 22 ignored tests are the live-tooling suites that require a downloaded
server, so a green `cargo test` does **not** exercise them
(`tests/optional_servers.rs`, `tests/kotlin_tooling.rs`,
`tests/lua_tooling.rs`, `tests/html_css_tooling.rs`).

---

## 2. Size: files over 800 lines and the 5 longest functions

### Files over 800 lines

| Lines | File |
| --- | --- |
| 8876 | `src/app/mod.rs` |
| 5372 | `src/language/tools.rs` |
| 2806 | `src/editor/document.rs` |
| 1288 | `src/ui/overlay.rs` |
| 1120 | `src/language/web/mod.rs` |
| 1110 | `src/language/lsp/mod.rs` |
| 922 | `src/project/create.rs` |
| 827 | `src/app/overlay.rs` |

(`src/ui/editor.rs` is next at 779, just under the line.)

### Five longest functions (non-test)

| Lines | Function | Location |
| --- | --- | --- |
| 281 | `builtin` (the command registry) | `src/commands/mod.rs:153` |
| 274 | `handle_overlay_key` | `src/app/mod.rs:1016` |
| 214 | `highlight` (web/TS tokenizer) | `src/language/web/mod.rs:304` |
| 207 | `handle_lsp_response` | `src/app/mod.rs:4349` |
| 192 | `apply_background_event` | `src/app/mod.rs:3096` |

`src/app/mod.rs` is the clear monolith: ~8.9k lines, three of the five longest
functions, and a test module starting at line 5972.

---

## 3a. Provisioning integrity

### The blanket claim

| Claim | Evidence | Verdict |
| --- | --- | --- |
| AGENTS.md: "Every managed install is verified and fail-closed." (`AGENTS.md:260`) | Five managed download plans pass `sha256: None` to `InstallStep::Download`: `jdtls_attempts` (`src/language/tools.rs:3580-3584`), `lua_ls_attempts` (`:3629-3633`), `kotlin_ls_attempts` (`:3672-3675`), and both OmniSharp downloads (`:3947-3951`, `:3963-3969`). None verify a digest. | **FALSE** |
| README "Managed toolchains" table is accurate about integrity | The rows for Java/jdtls, Kotlin, Lua and OmniSharp carry `—` under Integrity (`README.md:194-197`), which matches the code. | **TRUE** (the README is honest; AGENTS.md is not) |

So the two documents already contradict each other. Every *other* managed
download is verified; the gap is exactly the four components the README marks
with `—`.

### Integrity of every `InstallStep`

| `InstallStep` | Integrity check | Verified? | Location |
| --- | --- | --- | --- |
| `Download { sha256: Some(..) }` | SHA-256, removed on mismatch | yes | `src/language/tools.rs:1622-1631` |
| `Download { sha256: None }` | none | **no** | `:761-766` |
| `BobBuild` (Erlang/OTP, Elixir) | SHA-256 from `builds.txt`, fail-closed | yes | `:2361-2388` |
| `AdoptiumJdk` | checksum from the Adoptium API, fail-closed | yes | `:1639-1684` |
| `NodeRuntime` | `SHASUMS256.txt`, fail-closed | yes | `:1736-1768` |
| `DartSdk` | sibling `.sha256sum`, fail-closed | yes | `:1804-1839` |
| `DownloadGpg` (Swift) | detached GPG signature, isolated keyring, fail-closed | yes | `:1880-1961` |
| `GithubRelease` | SHA-256 `digest` from the GitHub API, fail-closed | yes | `:1967-1993` |
| `SwiftCompat` | local symlinks only, nothing downloaded | n/a | `:2003-2028` |
| `GoToolchain` | SHA-256 from `go.dev/dl/?mode=json`, fail-closed | yes | `:2089-2129` |
| `MakeExecutable` | chmod of a local file | n/a | `:2050-2068` |
| `Extract` | path-traversal check before unpack, fail-closed | n/a (safety, not provenance) | `:2408-2431` |

### Managed downloads with **no** integrity check

| Tool | URL / input | `sha256` | Evidence |
| --- | --- | --- | --- |
| `jdtls` | `download.eclipse.org/jdtls/snapshots/jdt-language-server-latest.tar.gz` (floating snapshot) | `None` | `:3556-3591` |
| `lua-language-server` | GitHub release `3.19.1` (pinned tag) | `None` | `:3615-3641` |
| `kotlin-language-server` | GitHub release `1.3.13` (pinned tag) | `None` | `:3648-3685` |
| `.NET SDK` | `dot.net/v1/dotnet-install.sh`, `--channel 10.0` | `None` | `:3947-3962` |
| `OmniSharp` | GitHub release `v2.0.0` tar.gz | `None` | `:3963-3969` |

### Floating / unpinned install inputs

| Input | Floating value | Evidence |
| --- | --- | --- |
| `go install` packages | `<pkg>@latest` (gopls, sqls, shfmt) | `:1014-1018` |
| npm packages | no version: `bash-language-server`, `typescript-language-server` + `typescript`, `vscode-langservers-extracted`, `prettier` | `:665-666`, `:700`, `:703` |
| pip package | no version: `python-lsp-server` (pipx / uv / venv / `--user`) | `:1179-1236` |
| gem | no version: `solargraph` | `:1160-1173` |
| cpan modules | no version: `PLS`, `Perl::LanguageServer` | `:3706-3750` |
| cargo crate | no version: `asm-lsp` (fallback) | `:885-901`, `:1118` |
| rustup components | follow the active toolchain channel | `:867-882` |
| `jdtls` | `-latest` snapshot URL | `:3557-3558` |
| `.NET` | floating `--channel 10.0` | `:3956-3957` |

Pinned-but-verified inputs for contrast: `NODE_VERSION` (`:37`),
`LUA_LS_VERSION` (`:42`), `KOTLIN_LS_VERSION` (`:46`), `DART_SDK_VERSION`
(`:50`), `SWIFT_VERSION` (`:58`), `ELIXIR_LS_VERSION` (`:62`),
`OTP_VERSION`/`ELIXIR_VERSION` (`:66-67`), `CPANM_VERSION` + `CPANM_SHA256`
(`:72-73`), `CLANGD_VERSION` (`:78`), `ASM_LSP_VERSION` (`:82`),
`PHPACTOR_VERSION` (`:86`). The Go toolchain and Adoptium JDK are intentionally
floating *versions* but **verified checksums**; `lua`, `kotlin` and `jdtls` are
pinned versions with **no** checksum.

---

## 3b. System binaries Koda shells out to

| Binary | Used for | Fallback | Evidence |
| --- | --- | --- | --- |
| `curl` | every managed download + all metadata/checksum fetches | none | `:1575`, `:1646`, `:1741`, `:1813`, `:1847` |
| `tar` | `.tar.gz`/`.tar.xz` listing + extraction | none (required for tar) | `:2436`, `:2494` |
| `unzip` | zip extractor #1 | yes → `bsdtar` → `python3` | `:2524-2533`, `:2582-2594` |
| `bsdtar` | zip extractor #2 (macOS default) | yes | `:2453`, `:2558` |
| `python3` | zip extractor #3, SHA-256, `jdtls` launcher, pylsp venv | partial | `:2529`, `:2589`, `:2687` |
| `gpg` | Swift signature verification | none (fails closed) | `:1881-1944` |
| `sha256sum` | hashing #1 | yes → `shasum` → `openssl` → `python3` | `:2661-2702` |
| `shasum` | hashing #2 (macOS) | yes | `:2661` |
| `openssl` | hashing #3 | yes | `:2673` |
| `git` | branch, status, diff, commit, staging | none; git UI degrades to empty | `src/git/mod.rs:144,162,178` |
| `sh` | rustup bootstrap, OTP `Install`, `dotnet-install.sh` | none (Unix-only) | `:948`, `:3853`, `:3953` |
| `rustup` / `cargo` | rust-analyzer/rustfmt, asm-lsp fallback | bootstrap via `sh.rustup.rs` | `:870`, `:889` |
| `go` | gopls/sqls/shfmt via `go install` | managed Go toolchain archive | `:1019`, `:986-1005` |
| `npm` / (`node`) | JS/TS/HTML/CSS/prettier servers | managed Node.js runtime | `:1253-1285` |
| `python3`/`python` | pylsp + jdtls prerequisite | none (reported) | `:1194`, `:617` |
| `perl` | PLS / `Perl::LanguageServer` via `cpanm` | none (reported) | `:3735`, `:3707` |
| `gem` | `solargraph` | none (reported) | `:1162` |
| `mix` / `elixir` / `erl` | ElixirLS build + `mix format` | managed toolchain | `:3874`, `:3895` |
| `java` | `jdtls` runtime | managed Adoptium JDK | `:3571-3578` |
| `dotnet` | OmniSharp runtime | managed .NET SDK | `:3952-3962` |

**No fallback at all:** `curl`, `tar`, `gpg`, `git`, `sh`. A missing `curl`
disables every managed download (`step_available` gates on `locate("curl")`,
`:2787-2799`). A missing `gpg` disables Swift only (fail-closed, `:2799`).

---

## 3c. Platform reality (Windows, macOS arm64)

| Claim | Evidence | Verdict |
| --- | --- | --- |
| CI covers Linux and macOS | `.github/workflows/ci.yml:13` matrix is `[ubuntu-latest, macos-latest]`; also Arch (glibc) and Alpine (musl) library jobs (`:30-61`) | **TRUE** |
| CI covers Windows | No Windows job; `matrix.os` has no Windows entry (`ci.yml:13`) | **FALSE** |
| There is *some* Windows code path | `cfg!(windows)` branches exist: venv `Scripts/` (`tools.rs:3980-3992`), managed node/npm layout (`:3205,3215`), rustup bootstrap disabled (`:960`), project-name validation (`project/create.rs:125-137`), formatter binary (`format.rs:207`) | **TRUE (partial)** |
| Windows is a supported, working target | Provisioning still shells out to `sh` for rustup/OTP/dotnet (`:948,3853,3953`); Swift compat is `cfg(unix)` (`:2036`); symlink/permission code is `cfg(unix)` (`:1927,2036,2054`); `format.rs:215` builds `PATH` with a hard-coded `:` separator, which is wrong on Windows. No Windows CI ever runs the app. | **FALSE** |
| macOS arm64 is supported by provisioning | Explicit `aarch64` asset arms exist for Go (`:2076`), Node (`:1710`), Dart (`:1792`), Adoptium (`:1697`), Lua (`:3601`), OmniSharp (`:3546`), asm-lsp (`:1088`); clangd has a mac asset (`:1039`). CI runs `macos-latest` (`ci.yml:13`); the image's CPU architecture is not verifiable from this repo. | **TRUE (provisioning); CI-arch unverified** |
| macOS gets Swift/Elixir provisioning | `bob_platform` returns `None` unless `OS == "linux"` (`:2307`); `swift_toolchain` returns `None` off Linux unless a native asset exists (`:2237`). So Elixir is Linux-only and Swift is discovery-only on macOS. | **TRUE (limitation)** |

**Bottom line:** the code is *compilable* on Windows in places but has no
end-to-end path — several install plans require a Unix `sh`, no CI runs it, and
one environment-construction site is Unix-only. Any Phase 3 distribution work
must treat Windows as **untested / likely broken** unless it is fixed first.

---

## 3d. Unchecked ROADMAP items

Every `[ ]` in `ROADMAP.md`, classified against the code:

| ROADMAP item (line) | Verdict | Evidence |
| --- | --- | --- |
| More complete Rust/Go tokenizer: strings, lifetimes, generics (`:180`) | **done (stale checkbox)** | Go handles interpreted/raw strings and runes (`src/language/go/mod.rs:240-258`); Rust handles raw strings and char-literals-but-not-lifetimes (`src/language/rust/mod.rs:221-240`). Generics are punctuation/operators by design. Tests exist (`go/mod.rs:417`). |
| Workspace → Project → Language environment → Files (`:204`) | **partial** | `Project`, `ProjectKind` and `Workspace` exist (`src/project/mod.rs:18,36`); no "language environment" layer found (grep found none). |
| Multi-project navigable workspace (`:208`) | **not started** | No multi-project surface; `Workspace` tracks one project. |
| Dedicated detection subsystem with pluggable signals + low-confidence user confirmation (`:210`) | **partial** | `language/detection/{engine,signals,confidence}.rs` exist (`engine.rs:98`). No user-confirmation prompt found (grep). |
| Optional theme system (`:330`) | **not started** | `ui/theme.rs` is a fixed Mellow mapping; no theme selection found. |
| File iconography (`:334`) | **not started** | No `icon` code in `src/ui`. |
| Interactive diff/merge view (`:335`) | **not started** | A read-only diff overlay exists; no interactive merge (only unrelated history coalescing). |
| Optional user configuration (`:352`) | **not started** | No config-file reading anywhere (grep for `config.toml`/`koda.toml` empty). |
| Plugin/provider API (`:354`) | **not started** | `commands/mod.rs:152` only has a *comment* about a future layer; no loader. |
| Remote development over SSH (`:355`) | **not started** | Only the OSC-52 "works over SSH" comment (`terminal/mod.rs:47`). |
| Debug adapter support (`:356`) | **not started** | No DAP code. |

---

## 3e. Architecture trees vs. reality

Named modules the task asked about, against the two trees:

| Module (real file) | In `AGENTS.md` tree | In `README.md` tree | Real |
| --- | --- | --- | --- |
| `language/lsp/` | missing | present (`README.md:567`) | `src/language/lsp/mod.rs` |
| `language/tools.rs` | missing | present (`README.md:568`) | exists (5372 lines) |
| `language/format.rs` | missing | missing | exists |
| `process.rs` | missing | missing | exists |
| `session.rs` | missing | present (`README.md:596`) | `src/lib.rs:34` |
| `background.rs` | missing | present (`README.md:598`) | `src/lib.rs:23` |

Also missing from **both** trees: `language/symbols.rs`,
`language/detection/{engine,signals,confidence}.rs` (summarised only), and
`src/recent.rs`, `src/regex.rs`, `src/search.rs` are present in both.

**Verdict:** `AGENTS.md`'s tree is stale (six missing modules); README's tree is
mostly current but omits `format.rs`, `process.rs` and `symbols.rs`.

---

## 4. Confirmed gaps to carry into later phases

1. **Phase 2 (trust):** pin and verify `jdtls` (replace the `-latest` snapshot
   with a released version), `lua-language-server`, `kotlin-language-server`,
   `.NET`/OmniSharp; add a single pinned-input table plus a test that fails on an
   unpinned/`sha256: None` managed download.
2. **Phase 2 (UX):** Language Setup shows no installed version or on-disk size
   and has no Update/Remove for managed tools (verified absent; README already
   promises only install).
3. **Windows:** either make it a real target (replace `sh` installers, fix the
   `PATH` separator, add CI) or explicitly declare it unsupported. Phase 3.1's
   distribution scope depends on this decision.
4. **`src/app/mod.rs` (8876 lines):** the module most in need of splitting in a
   future refactor; not part of Phases 0-2.
5. **Doc drift:** `AGENTS.md` architecture tree, its "every managed install is
   verified" claim, and `CHANGELOG.md`'s compare link
   (`https://example.com/koda/compare/main...HEAD`) all need correcting in
   Phase 1.

---

## Legend

- **true** — confirmed by code or a test that was read/run.
- **false** — contradicted by code or a test.
- **unverified** — could not be confirmed from this repository (no test, no
  live run); listed as an open question rather than a fact.
