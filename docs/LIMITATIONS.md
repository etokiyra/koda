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

- **Four managed downloads are not verified.** Eclipse JDT downloads the
  floating `jdtls` `-latest` snapshot, and `lua-language-server`,
  `kotlin-language-server` and OmniSharp (with `dotnet-install.sh`) download a
  pinned release with **no checksum or signature**. Every other fixed-artifact
  download is verified (see [DECISIONS.md](DECISIONS.md#provisioning)). Closing
  these four is the top item in [ROADMAP.md](../ROADMAP.md).
- **Package-manager installs are trusted.** `rustup`, `go install <pkg>@latest`,
  npm, pip, gem, cpan and `cargo install` fetch unpinned versions and delegate
  integrity to that manager; Koda adds no digest of its own.
- **A plan does not check every step before it starts.** `install()` runs a
  strategy's steps in order and does not consult `step_available` per step, so a
  component that cannot finish (for example the Swift compatibility layer on a
  distribution without `libxml2.so.2`) can still trigger its large download
  first. Verified in the Phase 0 audit: `install_check -- swift` began the
  ~1.1 GB download even though the compatibility step was unavailable.
- **Managed Swift is limited to swift.org's platforms.** The official Linux
  toolchains link against the distribution's libraries, so Koda installs one only
  where swift.org builds for the running release (Ubuntu, Debian, Fedora, Amazon
  Linux, RHEL). Elsewhere it uses the portable UBI10 build plus a compatibility
  layer on glibc Linux, and on musl it only discovers an existing toolchain.
  Swift downloads are large (~1.1 GB; ~3.5 GB extracted) and need disk for both.
  There is no free-space check, proxy handling or offline message before a large
  download.
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

- **No theme system.** The Mellow palette in `ui/theme.rs` is fixed; there is no
  runtime theme selection or user configuration.
- **No file iconography.** The tree uses text only.
- **No interactive diff/merge view.** The git diff is a read-only overlay; a
  conflicting-file merge view does not exist.
- **No mouse support.** Everything is keyboard-driven.

## Git

- **No merge tool and no hunk staging.** The changed-files list stages whole
  files; there is no per-hunk staging or gutter diff.
- **A non-UTF-8 path may not round-trip.** Recent/session stores serialize paths
  lossily.

## Platform

- **Windows is not supported.** There is no Windows CI; several provisioning
  plans shell out to a Unix `sh` (the `rustup` bootstrap, the Erlang `Install`
  script and `dotnet-install.sh`), the Swift compatibility layer is `cfg(unix)`,
  and `language/format.rs` builds a child `PATH` with a hard-coded `:` separator.
  The code compiles in places but has no end-to-end path.
- **Linux and macOS are supported.** CI builds and tests on Linux (glibc), in
  an Arch Linux container and on Alpine (musl), and on macOS. A job's presence
  is not proof it passed; see the "Platform and verification" matrix in the
  [README](../README.md#-platform-and-verification) for what was observed.
