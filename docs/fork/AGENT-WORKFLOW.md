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

`ca319dc` is the implementation/package base: the offline-validated,
five-provider candidate with fork identity, disabled self-update, and an
isolated data directory (see `docs/fork/DECISIONS.md#current-offline-only-status`).
This `tessera/offline-candidate` branch adds portable fork-development
metadata on top of that base: the agent workflow guide, decision/patch index,
Wayfinder map/tickets, handoff doc, tracked `.tpatch/` workspace, and the
Ubuntu/WSL plus live-GitLab validation record. Commit `ecae453` imported that
docs-only validation record; `c2a7554` is the exact source revision exercised
by it. Neither commit changes `src/` beyond `ca319dc`.

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

**Known environment-dependent failures**:

- `vcs::git::libgit2::tests::should_discover_worktree_with_relativeworktrees_extension`
  exercises relative worktree links introduced in Git 2.48 and fails on
  Ubuntu 24.04's Git 2.43 with "not in a git directory".
- On that same Git 2.43 host,
  `vcs::git::tests::default_preference_routes_reftable_repo_to_cli` also
  fails because the reftable backend integrated in Git 2.45 is unavailable
  and Git refuses the manually enabled repository extension.

Investigate any other failure. To confirm the supported suite on Git 2.43,
rerun with only those exact tests skipped; the real WSL baseline passed all
1,642 remaining fork tests. Git 2.48 is the oldest version that can exercise
both fixtures; put Git 2.48 or newer earlier on `PATH` to run the unfiltered
suite. Keep Ubuntu's security-patched system Git 2.43 available as the stock
distribution baseline rather than replacing it only to hide this portability
result. Official release notes:
[Git 2.45](https://github.com/git/git/blob/v2.45.0/Documentation/RelNotes/2.45.0.txt)
and
[Git 2.48](https://github.com/git/git/blob/v2.48.0/Documentation/RelNotes/2.48.0.txt).

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
5. `tpatch record <name> --commit-range <base>..<head>` — capture the
   **committed, tracked-only** diff between those two commits (untracked
   files are never included in a committed-range capture). Running
   `tpatch record <name>` with no range instead captures the current
   working tree, tracked **and** untracked.
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
integration and offline-candidate history through `ca319dc`, plus the
docs-only integration commits that bootstrapped this guide, the decision
index, and `.tpatch/` itself) carry `Tpatch-Feature: <name>` trailers as a
naming convention only — there is no corresponding
`.tpatch/features/<name>/` directory in this repo for them, and none should
be fabricated. See `docs/fork/PATCHES.md` for the full commit-range index.
Only run the full lifecycle above for *new* customizations from here
forward; expect their recipes to live under `.tpatch/features/<name>/` in
this repo.

## Wayfinder map/ticket/frontier conventions

`docs/wayfinder/map.md` and `docs/wayfinder/tickets/` hold one question or
prerequisite per ticket file, with frontmatter
`id`/`title`/`type`/`mode`/`status`/`owner`/`blocked_by`. The **frontier** is
the set of `open`, unclaimed tickets whose `blocked_by` entries are all
`closed`. Detailed resolutions live in the ticket's own `## Resolution`
section; the map's "Decisions so far" holds only a one-line linked gist —
never copy a resolution's full content into the map.

## Decisions and handoff docs

Durable fork decisions live in `docs/fork/DECISIONS.md` (one decision per
section, with enough rationale to stand on its own), the patch/commit index
is `docs/fork/PATCHES.md`, and the current state of in-flight fork work is
tracked in `docs/handoff/CURRENT.md`. These are self-contained fork-scoped
records: link to files inside this repo or public upstream URLs only, never
to a local research workspace or machine-specific path.

## WSL/provider release gates

`RELEASE.md`'s automated `bump-*` workflow (tag, crates.io publish, GitHub
Release, binaries) must **not** be triggered until the release is actually
ready: a real Ubuntu-under-WSL run has passed, and live GitHub/GitLab/Azure
DevOps/Gitea/Forgejo mutation validation has been approved and executed.
Anything produced before that (including `docs/offline-candidate/` archives)
is `OFFLINE_VALIDATED_ONLY`, not a release.

## Commit and safety rules

- Trailers: every commit needs **both** `Co-authored-by: Copilot
  <223556219+Copilot@users.noreply.github.com>` and a `Copilot-Session:
  <session-id>` trailer; commits produced through `tpatch land` also carry
  `Tpatch-Feature: <name>`.
- Never `git commit --amend`, `git push`, or `git tag` unless the user
  explicitly asks for that specific action.
- Never post remote review comments/data without an approved sandbox target
  (see Remote writes above).

## Evidence gap note

`docs/fork/DECISIONS.md`, `docs/fork/PATCHES.md`, `docs/wayfinder/`,
`docs/handoff/CURRENT.md`, and `.tpatch/` are all present in this repo as of
the `fork-ready-integration` branch. If any of them is missing on a branch
you are working from, that branch predates this integration — treat it as
not-yet-landed, not a broken link, and keep working from this file plus
`AGENTS.md`.
