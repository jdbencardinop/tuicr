# Fork agent workflow

Durable, compact rules for any coding agent (Copilot, Claude, etc.) working in
this repository: `jdbencardinop/tuicr`, an upstream-friendly fork of
[`agavra/tuicr`](https://github.com/agavra/tuicr). Read this before making
changes. `AGENTS.md` covers upstream architecture/conventions; this file
covers what is fork-specific.

## Why this fork exists

Adds durable provider-neutral review threads plus Azure DevOps/Gitea/Forgejo
adapters on top of upstream's local/GitHub/GitLab review flow. One canonical
`ReviewStore` (`src/review_store.rs`) — never add Hunk, Diffity, or a second
database as review state. AI (if any) stays external and optional: no
implicit model calls, no source upload.

## Current branch purpose

This branch integrates docs-only fork guidance (agent workflow, decision
index, tpatch bootstrap) onto the offline candidate at `ca319dc` before a
`fork-ready-integration` merge pushes a development branch to
`jdbencardinop/tuicr`. It does not change `src/`.

## Provider rules (no silent loss)

Every adapter operation is tagged `native`, `emulated` (exact substitute
documented), or `unsupported` (operation/provider/reason). No adapter may
silently drop a range, outcome, reply, or resolution, and no anchor may be
silently guessed, side-swapped, or moved to the nearest line. See
`docs/offline-candidate/PROVIDER-CAPABILITIES.md`.

## Remote writes

Never post/mutate real remote review data (comments, reviews, threads)
without an explicitly approved disposable sandbox target. Offline/mock and
already-verified live smoke tests are fine; new live writes are not.

## Build, lint, test

Run from the worktree root (`cargo` resolves `Cargo.toml` from `pwd`):

```
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --locked --lib            # full suite
cargo test --lib forge::azure        # scoped, e.g. one adapter module
```

**Known pre-existing failure**: `cargo test --lib` has exactly one
environmental failure, `vcs::git::libgit2::tests::should_discover_worktree_with_relativeworktrees_extension`
("not in a git directory"), unrelated to fork changes. Treat a run as clean
if this is the only failure; investigate any other failure or any change in
count.

## `tpatch` feature lifecycle

`tpatch` (`.tpatch/` workspace, added by the tpatch-bootstrap work) tracks
fork customizations as reviewable, replayable features:

1. `tpatch add <name>` — create the tracked feature request.
2. `tpatch analyze` → `define` → `explore` — understand, spec, and scope
   before editing (or `tpatch cycle` to run the sequence).
3. **Manual mode**: prefer doing analyze/define/explore/implement yourself
   and recording the result, over relying on an auto-detected local
   provider — this avoids incidental model calls on a customization task.
4. `implement` — generate a deterministic apply recipe; `apply` executes it.
5. `tpatch record <name> --commit-range <base>..<head>` — capture the actual
   tracked + untracked diff.
6. `tpatch test` — run the configured test command and record the result.
7. `tpatch verify` — check recipe/patch replay and dependency freshness.
8. `tpatch reconcile` — re-check a feature against a moved upstream base.
9. `tpatch land` — project one feature into a single Git commit carrying a
   `Tpatch-Feature: <name>` trailer.

Do not skip straight to `record`/`land` without analyze/define/explore for
anything non-trivial — the lifecycle is what keeps a patch independently
removable and reconcilable against upstream.

## Historical `Tpatch-Feature` trailers vs. new `.tpatch/` features

Commits made before this fork's `.tpatch/` workspace existed (the offline
integration and offline-candidate history, e.g. `ca319dc` and its ancestors)
already carry `Tpatch-Feature: <name>` trailers from feature work done in a
separate disposable research worktree. Those are historical record only —
there is no corresponding `.tpatch/features/<name>/` directory in *this*
repo for them. Once `.tpatch/` is bootstrapped here, only run the full
lifecycle above for *new* customizations, and expect their recipes to live
under `.tpatch/features/<name>/` in this repo.

## Wayfinder map/ticket/frontier conventions

If a `docs/fork/wayfinder/` map and tickets directory is present (added by
the decision-index work), it follows the same shape as the research
workspace: one question or prerequisite per ticket file, frontmatter
`id`/`title`/`type`/`mode`/`status`/`owner`/`blocked_by`. The **frontier** is
the set of `open`, unclaimed tickets whose `blocked_by` entries are all
`closed`. Detailed resolutions live in the ticket's own `## Resolution`
section; the map's "Decisions so far" holds only a one-line linked gist —
never copy a resolution's full content into the map.

## Decisions and handoff docs

Durable fork decisions and a patch/commit index live under
`docs/fork/decisions/` (or equivalent — see gap note below), and the current
state of in-flight fork work is tracked in `docs/handoff/CURRENT.md`. These
are minimal, fork-scoped summaries, not a copy of the full research
workspace at `/Users/jbencardino/Documents/Proyectos/diffreviewtui`.

## WSL/provider release gates

`RELEASE.md`'s automated `bump-*` workflow (tag, crates.io publish, GitHub
Release, binaries) must **not** be triggered until the release is actually
ready: a real Ubuntu-under-WSL run has passed, and live GitHub/GitLab/Azure
DevOps/Gitea/Forgejo mutation validation has been approved and executed.
Anything produced before that (including `docs/offline-candidate/` archives)
is `OFFLINE_VALIDATED_ONLY`, not a release.

## Commit and safety rules

- Trailers: every commit needs `Co-authored-by: Copilot
  <223556219+Copilot@users.noreply.github.com>`; commits produced through
  `tpatch land` also carry `Tpatch-Feature: <name>`.
- Never `git commit --amend`, `git push`, or `git tag` unless the user
  explicitly asks for that specific action.
- Never post remote review comments/data without an approved sandbox target
  (see Remote writes above).

## Evidence gap note

If any referenced path above (`docs/fork/decisions/`, `docs/fork/wayfinder/`,
`.tpatch/`) is missing, it means that part of the fork bootstrap has not
merged into your branch yet (e.g. mid cherry-pick/rebase) — treat it as
not-yet-landed, not a broken link, and keep working from this file plus
`AGENTS.md`.
