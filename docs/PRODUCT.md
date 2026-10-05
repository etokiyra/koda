# Koda's product direction

The authoritative statement of what Koda is for and who it is for. `AGENTS.md`
holds the engineering rules; this file holds the product direction;
[ROADMAP.md](../ROADMAP.md) turns it into prioritised work; the shipped state is
in [CHANGELOG.md](../CHANGELOG.md).

## The promise

> Install Koda. Open a project. Start coding.

Koda is a terminal-native IDE that detects the languages in a project and sets
up the tooling they need — without a configuration file, without asking the user
to understand language servers, and without taking over tools the user already
manages.

## The ideal experience

1. Install or download Koda (a prebuilt binary, or `cargo install`).
2. Launch it.
3. Open a file or a project.
4. Koda recognises the project's languages and offers to provision what is missing.
5. Completion, diagnostics, hover, navigation, rename, code actions and
   formatting work, backed by a language server when one is available and by
   Koda's built-in intelligence when it is not.
6. The user never writes a config file and never installs a language server by hand.
7. The user does not need to know which distribution they run or which package
   manager it uses.
8. Tools the user already installed are reused and never overwritten.
9. Koda stays lightweight, keyboard-first and transparent.
10. The handful of things people want to change can be changed from inside Koda.

## What "supported" means

A language is **supported** only when the claim matches what the code does; "an
LSP starts" is not enough. Koda uses four classes, defined in full in
[LANGUAGE-SUPPORT.md](LANGUAGE-SUPPORT.md#the-support-contract):

- **Complete** — detection, built-in offline editing, and a language server Koda
  provisions itself with nothing else required from the system.
- **Complete (prerequisite)** — the same, but one system runtime or tool must
  already exist (for example `php`, `ruby`/`gem` or `python3`); Koda names it
  and does not replace it.
- **Offline only** — detection and built-in offline editing; no language server
  (Markdown, JSON, TOML, YAML).
- **Platform-limited** — the server or toolchain is unavailable or unverified on
  some first-class platform; the limitation is stated per platform.

For a language to count as Complete it must provide:

- **Detection** — file-level signals identify it, with project context as
  corroboration.
- **Offline features** — highlighting and, where meaningful, structural
  diagnostics, symbols, completion, hover and navigation with no server and no
  network.
- **Provisioning** — Koda can obtain the server on each supported platform, or
  states precisely why it cannot.
- **Lifecycle and recovery** — the server is launched, restarted on failure and
  shut down cleanly; a missing prerequisite or no network leaves editing working
  and is explained.

Formatting, references, rename and code actions may legitimately be absent for a
language and still leave it Complete; the authoritative per-language state is
[LANGUAGE-SUPPORT.md](LANGUAGE-SUPPORT.md).

## Product principles

- **Zero configuration by default.** A fresh install needs no file. If
  customisation exists, it is reachable from inside Koda, and any on-disk
  settings file is an implementation detail the user never has to maintain.
- **Koda owns the tooling experience.** Detection, provisioning, launch,
  recovery and updates are Koda's job, not the user's.
- **Reuse, never hijack.** A user's or system's tool is preferred and is never
  modified, moved or deleted.
- **Fail closed and explain.** Downloads are verified, failures are bounded,
  partial state is cleaned up or clearly reported, and Koda never silently
  installs something it could not verify.
- **Distro-independent by construction.** Koda installs into user-local
  locations and does not depend on a distribution's package manager.
- **Terminal-native and light.** Transparency, keyboard-first, no opaque
  full-screen surfaces, no panel sprawl, no blocking the UI.
- **Breadth is not the goal; automatic support is.** Support fewer languages
  completely rather than many partially.
- **Be honest.** A feature is claimed only with code or a test behind it.

## Settings and themes

Koda is fully usable with no settings file. When a user wants to personalise it,
they open **Settings** from the command palette; there is no file to find or
edit.

- **Supported settings.** Theme, line numbers, animations, soft wrap, indent
  width (Auto or a fixed width), spaces vs tabs, inline diagnostics, and
  automatic completion. Each applies immediately and can be reset to its default.
- **Storage.** Choices are written on quit, and only once something changed, to
  `$XDG_STATE_HOME/koda/settings.json` (or
  `~/.local/state/koda/settings.json`). A missing file loads the defaults; a
  malformed file is set aside as `settings.json.corrupt` and the defaults load.
  The file is an implementation detail, never a requirement.
- **Themes.** Three are bundled and selectable at runtime: **Mellow** (the
  default), **Midnight** (high-contrast dark) and **Daylight** (light). They are
  data-driven from one semantic role set; users cannot yet add their own.

Settings are global. Koda does not create project-level configuration, and the
zero-configuration project experience is unchanged.

## Platform strategy

- **First-class:** Linux x86_64 (glibc), Linux x86_64 (musl), macOS x86_64,
  macOS arm64 — built and released for each (see the README install section).
- **Discovery-first:** other architectures (for example Linux aarch64) reuse a
  user's toolchain and provision managed tools only where upstream publishes a
  compatible artifact.
- **Documented per-tool platform support:** glibc-only bundles, Linux-only
  toolchains and required system libraries are stated rather than guessed.
- **Windows is out of scope.** Nothing in the current architecture makes it
  free, so Koda does not claim it and does not grow Windows-only paths.

## How a language should be added

Adding a language must stay cheap and uniform:

1. A `LanguageProvider` supplies detection descriptors and built-in
   (offline) features.
2. The language's tooling is described as data — the executable, its pinned
   source and digest, its launch arguments, its environment, its platform
   availability and its formatter — rather than as bespoke branches scattered
   through the UI.
3. The editor and the UI need no changes.

This is the target shape; today the provider half is in place and the
provisioning half is a `Tool` enum with per-tool plans. Making provisioning
descriptor-driven is a roadmap item, not a decision already made.

## The support strategy in one sentence

Prefer a small, self-contained, checksum-verified download from upstream over a
system package; reuse a user's existing tool when present; install under Koda's
own data directory; and never require the user to know how any of it works.

## Non-goals

Keep these explicit so the roadmap does not drift:

- Recreating VS Code in a terminal; panel-heavy layouts.
- A plugin marketplace or a general plugin/extension platform.
- Requiring a config file (or a config file's absence) to use a core feature.
- Supporting every language; chasing breadth over depth.
- Replacing the user's package manager or system tools.
- Windows support (for now).
- Bundling whole toolchains where a verified upstream download is smaller and
  safer.
