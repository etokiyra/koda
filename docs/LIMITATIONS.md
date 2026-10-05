# Known limitations

Grouped by subsystem. Each entry is a real, verified limitation, not a wish.
Open work that would remove one lives in [ROADMAP.md](../ROADMAP.md); this file
describes the current state.

---

## Editor

- **Non-UTF-8 files are refused.** Loading rejects binary, non-UTF-8 and
  oversized files rather than replacing bytes with U+FFFD and writing them back.
  There is no read-only or explicit-conversion path yet.
- **`perf` is untested at the extremes.** There is no budget test for a 50 MB
  file or for startup time; large-file behaviour is only known to be
  rope-backed, not measured.
- **Folding is not implemented.** Soft wrap is display-only; there is no code
  folding.

## Search and project

- **`.gitignore` discovery is capped.** At most 256 nested `.gitignore` files and
  4096 directories are read per project; in an enormous monorepo a deeper rule
  beyond the cap is not applied. The caps keep project open predictable.
- **The workspace model is single-project.** `Workspace` tracks one project and
  its language context; there is no navigable multi-project structure.

## LSP

- **Incremental sync is not implemented.** Koda always sends whole-document
  changes, which the protocol permits as the safe fallback even for servers that
  prefer incremental updates.
- **A server is launched per language, not per workspace root.** One server per
  `LanguageId` is tracked; a monorepo with several roots of the same language
  shares one server.
- **Code folding over LSP is not implemented** (see Editor).

## Provisioning

- **The verified cache has no eviction.** A verified download is stored under
  `<tools>/cache/<algorithm>-<digest>` and reused offline, so a reinstall or
  repair needs no network. Checksums and release metadata are cached by URL too,
  so metadata-driven installs can also fall back offline after their first
  successful fetch. Nothing evicts either cache, and removing a managed tool
  does not remove its cached archive, so they grow until the user clears them.
  The artifact cache uses hard links where the filesystem supports them, so it
  normally costs no extra disk over the downloads directory.
- **GPG-verified artifacts are not in the cache.** Swift's archive has no
  published digest to key on; it keeps the existing behaviour of reusing the
  already-verified file in the downloads directory (re-checked against its
  signature) rather than using the digest cache.
- **Atomicity is per step, not plan-wide.** A download is written to a temporary
  file and only renamed into place after verification, and an extraction stages
  into a sibling directory and promotes it with a rename, restoring the previous
  installation if promotion fails. But a plan that installs A and then fails on
  B can still leave A installed; Koda reports the incomplete plan and the next
  attempt recovers. There is no whole-plan transaction.
- **Pre-flight is best-effort.** `install()` keeps only strategies whose every
  step is available and checks the tools directory and disk space before any
  download, so an unrunnable plan (for example Swift without `libxml2.so.2`) or
  a short disk is reported first. Network reachability, proxy behaviour, archive
  contents and permissions below the tools directory remain runtime checks.
- **`rustup component add` remains a delegation.** Hex is pinned by exact version
  and rebar3 is a Koda-verified download. The only remaining floating
  provisioning input is rustup's component for the user's active toolchain:
  rustup chooses the component version and verifies toolchain artifacts against
  its signed manifests, and Koda does not fetch or pin it independently.
- **Swift's extracted size is not estimated.** The disk check uses the download
  estimate before an install and an archive's own size before extraction, but
  there is no separate estimate of the unpacked size (Swift is ~1.1 GB download
  to ~3.5 GB extracted), so a disk with room for the download but not the
  extraction is caught only during extraction.
- **Removing a shared managed component removes its siblings.** Update/Remove
  act on the first directory Koda owns beneath its tools root. Go-installed tools
  share `tools/gopath`, the npm servers share `tools/npm`, and the JDK and .NET
  runtimes are shared, so removing one tool can also remove another Koda tool.
  Koda never touches a user's own install.
- **Managed Swift is limited to swift.org's platforms.** The official Linux
  toolchains link against the distribution's libraries, so Koda installs one only
  where swift.org builds for the running release (Ubuntu, Debian, Fedora, Amazon
  Linux, RHEL). Elsewhere it uses the portable UBI10 build plus a compatibility
  layer on glibc Linux, and on musl it only discovers an existing toolchain.
  Swift downloads are large (~1.1 GB; ~3.5 GB extracted) and need disk for both.
  A network failure is reported with a hint (network, proxy or TLS) rather than a
  bare `curl` error.
- **`Perl::LanguageServer` cannot build on Perl ≥ 5.41.** Its `Coro` dependency
  (latest release 6.57, 2020) does not compile against Perl 5.42's changed
  `Time::HiRes` API. Verified on this host (Perl 5.42.2): the install fails with
  `Module 'Coro' is not installed`. Koda prefers **PLS**, which has no `Coro`
  dependency and installed successfully. A pre-installed `Perl::LanguageServer`
  is still discovered on any Perl.
- **PHP and Ruby still need their runtime.** Koda provisions `phpactor.phar` and
  an isolated `solargraph` gem, but not a PHP or Ruby interpreter: official PHP
  is source-only and the portable static builds publish no checksum, and Ruby has
  no official portable binary. Both runtimes are reused when installed and
  reported as a prerequisite otherwise.
- **`clangd` and `asm-lsp` prebuilt bundles are glibc-only.** Their prebuilt
  releases link against glibc and libstdc++, so Koda does not offer them on musl
  (Alpine/Void musl); a system clangd or the `cargo` fallback is used there. The
  official Go toolchain and Go-installed tools are static and work on musl.
- **Elixir/Erlang provisioning is Linux-only.** `bob_platform` returns `None`
  off Linux, so ElixirLS falls back to discovery on macOS.

## UI

- **No settings screen.** Only a few behaviours are toggles in the palette
  (animations, soft wrap, inline diagnostics); there is no persisted preference
  store and no Settings screen.
- **No theme selection.** The Mellow palette in `ui/theme.rs` is fixed; there
  are no alternative bundled themes.
- **Terminal coverage is narrow.** `Ctrl+Shift` chords rely on the kitty
  keyboard protocol (there is no fallback), there is no `NO_COLOR` or 16-colour
  mode, and the UI has not been checked on light backgrounds.
- **No file iconography.** The tree uses text only.
- **No interactive diff/merge view.** The git diff is a read-only overlay; a
  conflicting-file merge view does not exist.
- **No mouse support.** Everything is keyboard-driven.

## Language support

- **The authoritative contract is [LANGUAGE-SUPPORT.md](LANGUAGE-SUPPORT.md).**
  It records, per language, the built-in offline capabilities, the server and its
  pin and verification, the runtime prerequisite, the platforms and the
  verification status. This section lists only the known gaps.
- **Server-backed commands are gated on the built-in provider.** `Format
  Document`, `Rename Symbol` and `Code Actions` are LSP-backed, but the command
  palette enables them from the provider's declared capabilities. Only Rust and
  Go declare `Rename`/`Code Actions`, so the palette under-reports them for
  Python, Java, C#, Kotlin, Lua, PHP, Ruby and Swift even when their server
  supports them; the keybindings still work. Same for `Format Document` where
  the provider has no built-in formatter. See
  [LANGUAGE-SUPPORT.md](LANGUAGE-SUPPORT.md#known-inconsistency-the-capability-gate).
- **Built-in depth varies.** Several languages have built-in highlighting,
  symbols and completion but thin or no built-in diagnostics; their quality
  comes from the language server where one is installed.
- **Most servers are not live-verified beyond Linux glibc x86_64.** Only `gopls`,
  `clangd`, `asm-lsp`, `PLS`, `sqls`, Dart, ElixirLS, Lua, Kotlin, HTML and CSS
  have had a live handshake, and only on Linux glibc. Every server on Linux musl
  and macOS is build/unit-verified only.
- **Several servers are platform-limited.** Elixir is Linux-only; Swift is
  native or portable on glibc Linux and discovery-only elsewhere; Assembly has
  no prebuilt aarch64 Linux binary; `clangd` is withheld on musl. See
  [LANGUAGE-SUPPORT.md](LANGUAGE-SUPPORT.md#platform-notes).
- **PHP and Ruby need their runtime.** Koda provisions `phpactor.phar` and an
  isolated `solargraph` gem but not a PHP or Ruby interpreter (no verified
  portable upstream distribution), so their servers are Complete-with-a-
  prerequisite rather than zero-config. Built-in editing works offline with no
  runtime. See [Provisioning](#provisioning).

## Git

- **No merge tool and no hunk staging.** The changed-files list stages whole
  files; there is no per-hunk staging or gutter diff.
- **A non-UTF-8 path may not round-trip.** Recent/session stores serialize paths
  lossily.

## Platform

- **Windows is not supported.** There is no Windows CI; provisioning still
  shells out to a Unix `sh` for the Erlang/OTP `Install` script, the Swift
  compatibility layer is `cfg(unix)`, and `language/format.rs` builds a child
  `PATH` with a hard-coded `:` separator. The code compiles in places but has no
  end-to-end path.
- **Linux and macOS are supported.** CI builds and tests on Linux (glibc), in
  an Arch Linux container and on Alpine (musl), and on macOS. A job's presence
  is not proof it passed; see the "Platform and verification" matrix in the
  [README](../README.md#platform-and-verification) for what was observed.
