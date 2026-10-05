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

**Done (0.1.x).** A digest-keyed cache reuses verified downloads offline and
never bypasses verification; downloads and extractions are atomic per step and a
failed replacement restores the previous install; `install()` pre-flights the
plan and refuses an unrunnable plan or a short disk before downloading; failures
name the cause and the tool; Hex is pinned and rebar3 is checksum-verified by
Koda; fake-download tests cover cache hit/miss/corruption, mismatch, failed
download, staging cleanup and failed-promotion restore.

**Remaining.**
- `rustup component add` still follows the user's toolchain (the one documented
  delegation). Pin it, or keep the delegation and justify it.
- Rollback is per `Extract` step: a plan that installs A and fails on B may
  leave A. Decide whether a whole-plan transaction is worth it.
- Add an explicit offline/air-gapped message and a cache eviction policy.

**Acceptance for the remaining work.** No floating install input without a
documented reason; a multi-step plan leaves no misleading partial state and
reports what was installed; an offline attempt explains the cache/network state.

**Direction.** Pre-flight the whole plan; make the cache the primary artifact
store; treat a multi-step plan as a unit only if the rename-based promotion can
be extended safely.

**Depends on:** nothing. This is the foundation of the whole vision.

### 2. A complete supported-language matrix

**Goal.** Every language Koda knows has an explicit, honest contract, and every
limitation is deliberate and documented.

**Done (0.1.x).** [docs/LANGUAGE-SUPPORT.md](docs/LANGUAGE-SUPPORT.md) is the
authoritative contract and matrix: offline capabilities, server pin and
verification, runtime prerequisite, platform limits, class and verification
status for every language. PHP and Ruby are decided as Complete-with-a-
prerequisite (no verified portable runtime exists), and the Elixir, Swift,
Assembly and clangd platform limits are scoped. A test fails when a registered
language has no matrix row.

**Remaining.**
- Surface the per-language state inside Koda: `Language Setup` lists tools, not
  "what can I realistically expect for this language".
- Keep the matrix in sync as servers and platforms change.

**Direction.** Item 5 (project setup) consumes this contract; the in-app view
should be generated from the same data rather than a second hand-written table.

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

**Done (0.1.x).** Opening a project runs a bounded, `.gitignore`-aware scan for
its languages, plans each one (ready / needs install / needs a prerequisite /
unavailable), and offers a single **Project setup** summary with a
**Set up project** action and a **Later** dismissal. Selecting it installs every
missing managed tool through the existing provisioning path, one at a time, and
reports installed / ready / failed / needs-attention. Already-ready tooling is
never reinstalled, an independent language still succeeds if another fails, and
the per-language offer is suppressed for languages the project plan covers.
`Set Up Project…` in the palette opens it on demand.

**Remaining.**
- The welcome screen does not yet preview that Koda will set up tooling.

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
