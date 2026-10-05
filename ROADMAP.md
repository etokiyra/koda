# Roadmap

Koda is developed incrementally. This roadmap lists **open** work only; what has
shipped is in [CHANGELOG.md](CHANGELOG.md). It is product-oriented: every item
states the user-facing goal first, then how we will know it is done.

The product direction lives in [docs/PRODUCT.md](docs/PRODUCT.md). Read it
before adding an item here; if an item conflicts with those principles, the
principles win.

The guiding goal:

> Install Koda. Open a project. Start coding.

```
editor foundation  →  project/language detection  →  language intelligence  →  automatic tooling
      ✅ done                  ✅ done                    ✅ done                 ⏳ now
```

---

## Now

The work that most directly makes Koda feel like the zero-configuration IDE it
claims to be.

### 1. Trustworthy, unattended provisioning

**Goal.** Installing a language's tooling succeeds without the user's
intervention on any supported platform, or fails with a precise, actionable
reason — and never leaves a half-installed tool behind.

**Acceptance.**
- A plan refuses *before* downloading when a later step cannot run (the
  Swift-compatibility case in [LIMITATIONS.md](docs/LIMITATIONS.md#provisioning)).
- A failed attempt cleans up the partial files it created.
- A previously downloaded, checksum-verified artifact is reused, so an install
  or repair works offline.
- A network failure, a missing proxy and a full disk each produce a clear
  message rather than a raw `curl` error.
- No install input follows a floating channel (`rustup component add`,
  `mix local.hex`/`local.rebar` are the last two).
- Covered by fake-download tests proving the safe path.

**Direction.** Pre-flight the whole plan; make `downloads/` a verified cache
keyed by digest; roll back an attempt's destination directories on failure;
translate subprocess failures into user-facing text.

**Depends on:** nothing. This is the foundation of the whole vision.

### 2. A complete supported-language matrix

**Goal.** Every language in the README's "Supported languages" table has a
defined setup path, and every limitation is deliberate and documented.

**Acceptance.**
- One published matrix per language: built-in features, server, provisioning,
  formatting, updates/removal, offline behaviour and per-platform availability.
- The PHP and Ruby runtime question is decided (provision it, or state the
  prerequisite as first-class).
- Elixir on macOS, Swift on musl/non-Linux, and the glibc-only bundles are
  either given a path or explicitly scoped out.
- No cell is blank without an explanation.

**Direction.** Extend [docs/LIMITATIONS.md](docs/LIMITATIONS.md) with the
per-language view the README table summarizes.

**Depends on:** item 1 (so gaps can be closed, not just documented).

### 3. In-app Settings and themes

**Goal.** A user can change the handful of things that matter without ever
editing a file.

**Acceptance.**
- A **Settings** screen (from the palette) changes at least: theme, animations,
  soft wrap, inline diagnostics, default indentation, completion behaviour and
  keybindings.
- Choices persist across launches; a fresh install still needs no file.
- At least two bundled themes beyond the default Mellow, selectable at runtime.

**Direction.** A settings store under Koda's data directory, a small set of
typed preferences, and a TUI that writes it. The file is an implementation
detail, never a requirement.

**Depends on:** nothing.

### 4. Terminal compatibility

**Goal.** Koda is readable and fully usable on the terminals and setups people
actually use.

**Acceptance.**
- `Ctrl+Shift` chords work when the kitty keyboard protocol is unavailable
  (verified under `tmux`), with a documented fallback.
- `NO_COLOR` and a 16-colour fallback produce a legible UI.
- Koda is legible on a light terminal background.

**Direction.** Detect capability once at startup and expose a fallback keymap;
route colour through the theme so a fallback theme can be chosen.

**Depends on:** item 3 for the fallback theme.

### 5. First-run and project setup

**Goal.** The first five minutes explain Koda and get a project working in one
step.

**Acceptance.**
- Opening a project detects its language(s), lists missing tooling and offers a
  single **Set up this project** action that provisions all of it.
- Dismissing changes nothing and leaves built-in intelligence working.
- The welcome screen communicates what Koda will do before it does it.

**Direction.** Reuse language detection and the provisioning queue; batch the
offers per project instead of per language.

**Depends on:** items 1 and 2.

### 6. Packaging polish

**Goal.** Install Koda without a Rust toolchain, and update it easily.

**Acceptance.**
- `cargo install koda` works from crates.io metadata.
- A one-line installer (`curl … | sh`) installs the right prebuilt binary.
- The README install section stays the single authoritative source.

**Depends on:** the release workflow (shipped in 0.1.0).

---

## Next

Valuable, but not what defines the first impression.

- **Code folding.** *Goal:* collapse and expand ranges (`za`/`zo`/`zc`).
  *Acceptance:* ranges fold from LSP or blank-line heuristics and survive edits.
- **Run and test tasks.** *Goal:* run a project's own command with no config.
  *Acceptance:* the task is detected per project kind, runs with bounded output,
  and `file:line` in its output jumps to the source.
- **Managed-tool lifecycle.** *Goal:* keep provisioned tools current safely.
  *Acceptance:* Update re-resolves the pinned version, an optional auto-update
  preference exists, and Remove never touches a user/system install.
- **Detection confirmation.** *Goal:* resolve genuinely ambiguous files.
  *Acceptance:* a low-confidence file asks once and remembers the answer.
- **Performance budgets.** *Goal:* Koda stays fast on real files.
  *Acceptance:* budget tests assert startup time and opening a 50 MB file, and
  regressions fail CI.
- **Non-UTF-8 files.** *Goal:* open them instead of refusing. *Acceptance:* a
  non-UTF-8 file opens read-only, with an explicit conversion option.
- **Descriptor-driven provisioning.** *Goal:* add a language without new
  bespoke branches. *Acceptance:* a language's tooling is declared as data; the
  editor, UI and provisioning code need no per-language changes.
- **Git gutter.** *Goal:* see changes in the margin. *Acceptance:* added,
  modified and removed markers plus next/previous hunk, computed off the UI
  thread.

---

## Later

Real, but deferred; none of them is required for the core promise.

- **Multi-project workspace.** Open and navigate several projects independently.
- **LSP incremental sync.** Send incremental changes when a server prefers them.
- **Mouse support** (optional, never required): click, drag, wheel, tab/tree clicks.
- **File iconography** that respects monochrome terminals.
- **Interactive diff/merge** for conflicting files.
- **Remote development over SSH.**
- **Debug adapter support.**
- **Highlighting engine comparison.** Measure the hand-written tokenizers
  against tree-sitter and record a recommendation in
  [docs/DECISIONS.md](docs/DECISIONS.md); do not migrate without an explicit OK.
- **Plugin/provider API.** Only with a design that cannot bloat Koda; see the
  non-goals.

---

## Non-goals

These are out of scope by design, not merely unscheduled:

- Recreating VS Code in a terminal, or a panel-heavy layout.
- A plugin marketplace or a general extension platform.
- Requiring a configuration file — or its absence — for a core feature.
- Supporting every language; breadth over depth.
- Replacing the user's package manager or system tools.
- Windows support, unless the architecture makes it essentially free (it does not).
- Bundling whole toolchains where a smaller, verified upstream download exists.

See [docs/PRODUCT.md](docs/PRODUCT.md#non-goals) for the longer statement.

---

## Completed

Shipped work lives in [CHANGELOG.md](CHANGELOG.md). Highlights: the rope editor
and UI, language detection and built-in providers, the LSP client and
server-backed features, pinned and verified tool provisioning with Language
Setup, git integration, session persistence, CI, and prebuilt 0.1.0 release
binaries.
