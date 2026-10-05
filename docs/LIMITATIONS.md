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

- **No offline or cached install.** Downloads are fetched on every attempt;
  only the Swift signature path reuses an already-verified archive. A machine
  with no network cannot install or repair a managed tool even if it downloaded
  the same artifact before.
- **No rollback.** A failed attempt can leave a partially extracted tree under
  Koda's data directory. A retry overwrites it, but nothing is cleaned up
  proactively.
- **Two package-manager inputs are still delegations.** Every explicit package
  install names an exact version and delegates integrity to its manager:
  `rustup component add` follows the user's toolchain channel, and `mix
  local.hex`/`local.rebar` fetch Hex's own signed archive. Koda adds no digest of
  its own for these two; every other managed install input is pinned and
  verified (see [DECISIONS.md](DECISIONS.md#provisioning)).
- **A plan does not check every step before it starts.** `install()` runs a
  strategy's steps in order and does not consult `step_available` per step, so a
  component that cannot finish (for example the Swift compatibility layer on a
  distribution without `libxml2.so.2`) can still trigger its large download
  first. Verified in the Phase 0 audit: `install_check -- swift` began the
  ~1.1 GB download even though the compatibility step was unavailable. The
  disk-space check now runs first, which bounds the damage, but the download is
  still attempted.
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
  There is no dedicated offline message before a download; a network failure is
  reported as a curl error.
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

- **The per-language contract is not surfaced.** `Language Setup` lists tools,
  but Koda does not show a per-language view of what works offline, what needs a
  server, and what a platform cannot provide. The README "Supported languages"
  table is the only such view, and it is not machine-checked.
- **Built-in depth varies.** Several languages have built-in highlighting,
  symbols and completion but thin or no built-in diagnostics; their quality
  comes from the language server where one is installed. See the per-tool and
  per-platform caveats in [Provisioning](#provisioning).

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
