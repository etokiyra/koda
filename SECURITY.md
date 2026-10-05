# Security

## Reporting a vulnerability

Please report security issues privately through the repository's **Security →
Report a vulnerability** (GitHub security advisories) rather than a public
issue. Include the version, platform, a reproduction, and the impact. We will
acknowledge and investigate as soon as we can.

## Supported versions

Koda is pre-1.0. Security fixes are made on `master`; there are no maintained
release branches yet.

## Managed-download trust model

Koda installs language tools for the user. This section states exactly what it
downloads, from where, and how the download is verified, so the trust boundary
is explicit.

### Verified downloads

Where upstream publishes a checksum or signature, Koda verifies it before the
archive is used, and **fails closed** — a missing check or a mismatch aborts the
install and leaves nothing behind.

| Component | Source | Verification |
| --- | --- | --- |
| Go toolchain | `go.dev/dl` | SHA-256 from the release JSON |
| clangd | `github.com/clangd/clangd` | SHA-256 asset digest from the GitHub API |
| asm-lsp | `github.com/bergercookie/asm-lsp` | SHA-256 asset digest from the GitHub API |
| phpactor | `github.com/phpactor/phpactor` | SHA-256 asset digest from the GitHub API |
| ElixirLS | `github.com/elixir-lsp/elixir-ls` | SHA-256 asset digest from the GitHub API |
| Dart SDK | `storage.googleapis.com/dart-archive` | sibling `.sha256sum` |
| Node.js | `nodejs.org` | `SHASUMS256.txt` |
| Eclipse Adoptium JDK | `api.adoptium.net` | checksum from the same API response |
| Erlang/OTP, Elixir | `builds.hex.pm` | SHA-256 from `builds.txt` |
| Swift toolchain | `download.swift.org` | detached GPG signature, verified against swift.org's published keys in a **private keyring** |
| `App::cpanminus` | `cpan.metacpan.org` | a SHA-256 pinned in Koda's source |

### Downloads that are **not** verified yet

These download a pinned release but with no checksum or signature. They are a
known gap, tracked in [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md#provisioning):

| Component | Source |
| --- | --- |
| Eclipse JDT (`jdtls`) | `download.eclipse.org` — a floating `-latest` snapshot |
| `lua-language-server` | `github.com/LuaLS/lua-language-server` |
| `kotlin-language-server` | `github.com/fwcd/kotlin-language-server` |
| OmniSharp and `dotnet-install.sh` | GitHub release and `dot.net` |

### Package-manager installs

`rustup`, `go install <pkg>@latest`, npm, pip, gem, cpan and `cargo install`
fetch unpinned versions and delegate integrity to that package manager (its
TLS and any signing it performs). Koda adds no digest of its own for these. The
Rust toolchain additionally bootstraps the official `rustup` installer over
HTTPS from `sh.rustup.rs`.

### Enforced guarantees

- **No checksum, no install** where upstream publishes one: verification is
  fail-closed, and a hash that is not 64 hex characters is rejected.
- **No system writes.** Installations target a user-writable directory Koda
  manages (or the user's own `~/.cargo`, `~/.local`), so `sudo` is never
  required and a system-owned prefix cannot be corrupted.
- **Isolated subprocesses.** Installs run from Koda's own data directory, not
  the user's project, so a project-local `.npmrc` or similar cannot hijack a
  package manager. Commands and downloads are time- and output-bounded.
- **Safe extraction.** Before unpacking, Koda lists an archive's entries and
  refuses absolute paths or `..` components, failing closed if it cannot list.
- **Serialised installs.** A shared managed directory is guarded by an advisory
  lock with stale-lock recovery and a nonce, so two instances cannot corrupt the
  same prefix.
- **TLS.** Downloads use `curl` with `--proto =https` and TLS 1.2+, and abort on
  HTTP failure.

### What this model does not cover

- **Upstream or transport compromise.** Koda trusts HTTPS and the integrity data
  published by each upstream; a compromised upstream that also publishes a
  matching checksum is not defended against.
- **The four unverified downloads.** Their content is trusted on the strength of
  HTTPS and the pinned URL alone.
- **Windows.** Windows is not supported, so no security guarantees are claimed
  there.

## Editor and data-handling notes

- Koda does not send project contents anywhere. Language servers run locally and
  Koda talks to them over stdio; the only network traffic is tool provisioning.
- Saves are atomic: a sibling temporary file is written and renamed over the
  original. Filesystem rename/copy never overwrites an existing destination.
- Loading refuses binary, non-UTF-8 and oversized files rather than replacing
  bytes and writing them back.
- The clipboard uses the terminal's OSC 52 sequence; Koda keeps an internal
  clipboard as a fallback.
