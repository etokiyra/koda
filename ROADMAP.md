# Roadmap

Koda is developed incrementally. This roadmap lists **open** work only; what has
shipped is in [CHANGELOG.md](CHANGELOG.md). Each item carries one line of
acceptance criteria. Nothing here is a commitment to a date.

The guiding goal:

> Install Koda. Run `koda .`. Open a file. Start coding. It just works.

Status of the foundation (see the [README](README.md) and
[docs/LIMITATIONS.md](docs/LIMITATIONS.md) for the current state):

```
editor foundation  →  project/language detection  →  language intelligence
      ✅ done                  ✅ done                    ✅ done
```

---

## Now

- [ ] **Close the provisioning integrity gap.** Pin and verify the four managed
      downloads that have no checksum today: replace the floating `jdtls`
      snapshot with a released version, and verify `lua-language-server`,
      `kotlin-language-server` and OmniSharp (plus `dotnet-install.sh`).
      *Acceptance:* every managed download input has a pinned version and a
      verified digest, and a test fails if any install input is unpinned or
      unverified.
- [ ] **Split `src/app/mod.rs`** (8876 lines) into focused modules, mechanical
      moves only, one extraction per commit, no behaviour change. The proposed
      split is in
      [docs/DECISIONS.md](docs/DECISIONS.md#proposed-splitting-srcappmodrs).
      *Acceptance:* no source file exceeds 800 lines and the full test suite
      passes after every commit.
- [ ] **Give Language Setup version, size, Update and Remove** for
      Koda-managed tools only; never touch the user's own installs.
      *Acceptance:* each managed tool shows its installed version and on-disk
      size, and can be updated or removed without touching a discovery-only
      install.

## Next

- [ ] **Distribution.** A release workflow with prebuilt binaries (Linux gnu and
      musl, macOS x64 and arm64), an install one-liner, and crates.io-ready
      metadata.
      *Acceptance:* a user with no Rust toolchain can install and run `koda`.
- [ ] **Run and test tasks.** Detect runnable tasks per project kind through a
      provider method (`cargo run`/`test`, `go test`, `npm test`, `pytest`, …)
      with no config file.
      *Acceptance:* the detected task runs in a bounded output panel and
      `file:line` in its output jumps to the source.
- [ ] **Robustness.** Property/fuzz tests for the rope buffer, layout, regex and
      `.gitignore` matcher; a performance budget (open a 50 MB file, startup
      time); and an explicit way to open non-UTF-8 files (read-only or with a
      conversion) instead of refusing them.
      *Acceptance:* fuzz/property tests run in CI, the perf budget is asserted,
      and a non-UTF-8 file opens without data loss.
- [ ] **LSP incremental sync, then code folding.**
      *Acceptance:* servers that prefer incremental sync receive incremental
      changes, and ranges can be folded and unfolded.
- [ ] **Terminal compatibility.** Fallback bindings for `Ctrl+Shift` chords when
      the kitty protocol is unavailable (verified under `tmux`), `NO_COLOR` and
      16-colour fallbacks, and a layout readable on light terminal backgrounds.
      *Acceptance:* the chords work under `tmux` with no kitty support, and Koda
      is legible with `NO_COLOR=1` and on a 16-colour, light-background terminal.
- [ ] **Git gutter.** Added/modified/removed markers in the gutter, plus
      next/previous hunk, computed on the background worker.
      *Acceptance:* markers track edits without blocking the UI and hunk
      navigation moves between changes.
- [ ] **Highlighting comparison.** Measure the hand-written Rust/Go tokenizer
      against tree-sitter (binary size, startup, build time, accuracy on this
      repository's own source) and write the comparison in
      [docs/DECISIONS.md](docs/DECISIONS.md).
      *Acceptance:* the comparison is recorded with numbers and a single
      recommendation. Do not migrate without the maintainer's OK.

## Later

- [ ] **Multi-project workspace.** A `Workspace → Project → Language
      environment → Files` model surfacing several projects as one navigable
      structure.
      *Acceptance:* two projects in one tree can be opened and navigated
      independently.
- [ ] **Detection confirmation.** When confidence is low, ask the user to
      confirm the detected language once.
      *Acceptance:* a low-confidence file prompts, and the choice is remembered
      for the session.
- [ ] **Optional user configuration** (theme, keybindings, editor behaviour),
      with excellent defaults so a fresh install never needs a config file.
      *Acceptance:* a config file can override the defaults, and its absence
      changes nothing.
- [ ] **Mouse support** (optional, never required): click to place the cursor,
      drag to select, wheel scroll, click tabs and tree rows.
      *Acceptance:* mouse actions work and the keyboard remains fully sufficient.
- [ ] **Theme system.** A subtle, optional theme selection that works with no
      config by default.
      *Acceptance:* at least one alternative theme can be selected at runtime.
- [ ] **File iconography** that respects monochrome terminals.
      *Acceptance:* the tree distinguishes common file kinds without relying on
      colour alone.
- [ ] **Interactive diff/merge view** for conflicting files.
      *Acceptance:* a conflicting file can be resolved and written back.
- [ ] **Plugin/provider API** for third-party languages.
      *Acceptance:* an out-of-tree provider can be registered without editing
      Koda's source.
- [ ] **Remote development over SSH.**
      *Acceptance:* a project on a remote host can be opened and edited.
- [ ] **Debug adapter support.**
      *Acceptance:* a breakpoint can be set and hit through a DAP server.

---

## Non-goals

- Recreating VS Code in a terminal.
- Filling the screen with panels and controls.
- Publishing a marketplace of half-working plugins.
- Requiring the user to understand LSP, install language servers, or write
  configuration before they can code.
- Windows support (Linux and macOS only for now).
