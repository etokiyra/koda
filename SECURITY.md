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

Every managed download is verified before it is used, and **fails closed** — a
missing check or a mismatch aborts the install and removes the bad file.

| Component | Source | Verification |
| --- | --- | --- |
| Go toolchain | `go.dev/dl` | SHA-256 from the release JSON |
| clangd | `github.com/clangd/clangd` | SHA-256 asset digest from the GitHub API |
| asm-lsp | `github.com/bergercookie/asm-lsp` | SHA-256 asset digest from the GitHub API |
| phpactor | `github.com/phpactor/phpactor` | SHA-256 asset digest from the GitHub API |
| ElixirLS | `github.com/elixir-lsp/elixir-ls` | SHA-256 asset digest from the GitHub API |
| `lua-language-server` | `github.com/LuaLS/lua-language-server` | SHA-256 asset digest from the GitHub API |
| OmniSharp | `github.com/OmniSharp/omnisharp-roslyn` | SHA-256 asset digest from the GitHub API |
| Eclipse JDT (`jdtls`) | `download.eclipse.org` milestones | SHA-256 pinned from the published `.sha256` |
| `kotlin-language-server` | `github.com/fwcd/kotlin-language-server` | SHA-256 pinned in Koda's source (GitHub publishes no asset digest) |
| Dart SDK | `storage.googleapis.com/dart-archive` | sibling `.sha256sum` |
| Node.js | `nodejs.org` | `SHASUMS256.txt` |
| Eclipse Adoptium JDK | `api.adoptium.net` | checksum from the same API response, for a pinned release |
| .NET SDK | `builds.dotnet.microsoft.com` | SHA-512 from the channel's `releases.json` |
| Erlang/OTP, Elixir | `builds.hex.pm` | SHA-256 from `builds.txt` |
| `rustup-init` | `static.rust-lang.org` | SHA-256 beside the versioned binary |
| Swift toolchain | `download.swift.org` | detached GPG signature, verified against swift.org's published keys in a **private keyring** |
| `App::cpanminus` | `cpan.metacpan.org` | a SHA-256 pinned in Koda's source |

The two Koda-pinned SHA-256 values (`kotlin-language-server` and
`App::cpanminus`) are recorded from a reviewed download of the exact immutable
release asset, because the upstreams publish no digest of their own. A replaced
asset therefore fails closed rather than installing silently.

### Package-manager installs

`go install`, npm, pip, gem, cpan and `cargo install` are invoked with an exact
version, so an install is reproducible, and integrity is delegated to that
package manager (its TLS and any signing it performs). Two inputs remain
manager-delegated with no fixed version: `rustup component add` (which follows
the user's toolchain channel) and `mix local.hex`/`local.rebar` (Hex's own
signed archive). See [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md#provisioning).

### Enforced guarantees

- **No digest, no install.** A `Download` step carries a required SHA-256, and
  every other download step verifies a checksum, signature or upstream digest.
  Verification is fail-closed, and a hash that is not the expected length of hex
  is rejected.
- **No system writes.** Installations target a user-writable directory Koda
  manages (or the user's own `~/.cargo`, `~/.local`), so `sudo` is never
  required and a system-owned prefix cannot be corrupted.
- **Isolated subprocesses.** Installs run from Koda's own data directory, not
  the user's project, so a project-local `.npmrc` or similar cannot hijack a
  package manager. Commands and downloads are time- and output-bounded.
- **Safe extraction.** Before unpacking, Koda lists an archive's entries and
  refuses absolute paths or `..` components, failing closed if it cannot list.
- **Disk-space check.** Before a large install Koda refuses to start when the
  target filesystem cannot hold the estimated download, and before extraction it
  checks the archive's own size.
- **Proxy-aware.** The selected `HTTPS_PROXY`/`ALL_PROXY`/`HTTP_PROXY` value is
  forwarded to `curl`, which also honours these variables itself.
- **Serialised installs.** A shared managed directory is guarded by an advisory
  lock with stale-lock recovery and a nonce, so two instances cannot corrupt the
  same prefix.
- **TLS.** Downloads use `curl` with `--proto =https` and TLS 1.2+, and abort on
  HTTP failure.

### What this model does not cover

- **Upstream or transport compromise.** Koda trusts HTTPS and the integrity data
  published by each upstream; a compromised upstream that also publishes a
  matching checksum is not defended against. For the two Koda-pinned SHA-256
  values, the first download is trust-on-first-use of a reviewed artifact.
- **Manager-delegated installs.** `rustup component add` and `mix local.hex`
  trust those managers' own trust model.
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
