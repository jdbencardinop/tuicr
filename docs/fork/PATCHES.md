# Fork Patch Index

Historical index of the `tpatch`-tracked features that make up this fork,
derived from committed `Tpatch-Feature:` trailers in `git log`. This is the
durable record; do not hand-edit the table below without re-deriving it from
`git log` (see "How this list was produced").

Every commit below carries a `Tpatch-Feature: <name>` Git trailer. Historical
entries through `fork-tpatch-bootstrap` used that trailer as a naming
convention and predate this repo's tracked `.tpatch/` workspace; do not create
feature directories for them after the fact. New customizations beginning with
`classify-gitlab-range-anchors` use the full tracked lifecycle and retain their
request/spec/exploration/recipe/evidence under `.tpatch/features/<name>/`.

## Honesty note on `.tpatch/` metadata

`tpatch`'s own per-feature state (`.tpatch/features/<slug>/status.json`) for
work done in a sibling worktree lives under that worktree's own `.tpatch/`
directory, which is local, uncommitted, and worktree-specific — it does not
travel with `git clone`/`git log` and can vanish if that worktree is pruned.
The table below treats the committed `Tpatch-Feature:` trailer as the only
guaranteed-durable identifier, and reports local `.tpatch` state only where it
was directly inspected in a still-present sibling worktree at the time this
file was written. Where it was not (re)inspected, this is stated explicitly
rather than assumed. This repo's own `.tpatch/` workspace is tracked from `24376a6` onward. Its
current feature state is portable in Git and is reported directly below; see
"Relationship to this repo's `.tpatch/` workspace".

## Source vs. reconciled commits

Every feature below was built directly in this fork's own commit lineage
**except Azure DevOps**, which was implemented in a separate sibling
repository/worktree (branch `azure-adapter-offline`) and later re-applied
("reconciled") onto this lineage as new commits with matching messages but
different hashes. Both ranges are listed for that feature; the source range
is not reachable from this branch's history, only the reconciled range is.

## Index

| Tpatch-Feature slug | Commit range (this lineage) | Base | Local `.tpatch` state (uncommitted, best-effort) | Validation state | Upstream target |
|---|---|---|---|---|---|
| `expose-review-comment-authors` | `6ea2048` (1 commit) | `f92502b` (upstream branch point) | not (re)inspected this pass | Implemented, targeted tests pass, verified via a 5-insertion/1-deletion diff | **Ready to propose** — see `docs/wayfinder/tickets/upstream-and-reconcile.md` |
| `editable-provider-neutral-threads` | `a65737c..60115b0` (5 commits) | `fb7d430` | not (re)inspected this pass | Implemented against frozen contract fixtures (migration, replies, resolution, authorship, provider mappings, cross-head identity, anchor states) | Fork-only for now; upstream split TBD |
| `provider-capabilities` | `9acea98..b61bdbb` (4 commits) | `60115b0` | not (re)inspected this pass | Implemented; registry/profiles/dry-run planner verified against fake provider profiles | Fork-only for now; upstream split TBD |
| `gitea-forgejo-adapter` | `795f2bc..7a61137` (4 commits) | `b61bdbb` | Inspected in sibling worktree `gitea-forgejo-adapter`: `state: applied`, last `verify` passed | Live-verified against disposable local **Gitea 1.24** and **Forgejo 16**; reply/resolve/edit routes confirmed unsupported (405 on stable Swagger) | Not proposed; new adapter, candidate after thread contract stabilizes |
| `azure-devops-adapter-offline` | **Source** (sibling repo, not on this branch): `db10560..50eaad5` (4 commits, branch `azure-adapter-offline`). **Reconciled onto this lineage**: `7ee96d2..7078b82` (4 commits, same messages/order, different hashes) | `7a61137` | Inspected in sibling worktree `azure-adapter-offline`: `state: applied` (local/uncommitted; disappears if that worktree is removed) | Offline/mock-verified; live create/reply/status and stale classification pass, with native-anchor preservation supplied by the later tracked feature | Not proposed; remaining live gap is TUI inspection before cleanup |
| `fix-azure-connection-data-version` | `50c4580` | `50c4580` | `tpatch verify` exact landing evidence | Live viewer lookup and all native non-draft vote states pass: approve, suggestions, wait, reject, reset | Candidate for the Azure adapter upstream split; preserves stable 7.1 on all Git endpoints |
| `preserve-azure-native-anchors` | `c3d6dde` | `44f5576` | Tracked feature: `state: applied`; landing evidence exact at `c3d6dde` | Live-complete for anchors: current and stale native context persists across ReviewStore reload/re-import, and the real TUI renders the shifted root/reply as `outdated · locally stale` | Candidate for the Azure adapter upstream split |
| `fix-github-head-refresh` | `0ddd120` | `6a84091` | Tracked feature: `state: applied`; recipe replay and preimage verification pass | Live-complete on Ubuntu 24.04/WSL2: the 14-file, 1,005-line cumulative diff and one provider discussion relocated from line 42 to 47 survive `:e` and fresh restart | Fork-only for now; upstream split TBD |
| `tui-thread-integration` | `732745c..9a965a2` (5 commits) | `7a61137` | not (re)inspected this pass | Implemented; canonical durable-thread TUI rendering, remote-overlay, reply-splice ordering, race-condition fix | Fork-only; TUI rendering is fork-specific by nature |
| `github-gitlab-thread-parity-offline` | `cc38057..7b086df` (3 commits) | `9a965a2` | not (re)inspected this pass | Offline/mock parity complete (durable create/reply/resolve, REST/GraphQL ID mapping, resume/idempotency, head-update handling); GitLab live-passes, while GitHub live validation found durable TUI publication and post-head-update rendering failures | N/A — parity work on existing adapters, not a new upstream-proposable feature; see `docs/wayfinder/tickets/validate-live-provider-parity.md` |
| `offline-integration` | `1838181..d8b1f63` (2 commits) | `7078b82` | not (re)inspected this pass | Complete — integrates the Azure reconciliation and the five-provider offline source into one lineage | N/A — integration glue, not an upstream-proposable feature |
| `offline-release-candidate` | `0ef4dcf..ca319dc` (5 commits) | `d8b1f63` | not (re)inspected this pass | `OFFLINE_VALIDATED_ONLY` — packaged, checksummed, secret-scanned macOS/Linux x86_64 archives; not a tagged/pushed release | **Never upstream** — fork identity, disabled self-update, isolated data dir, and packaging secret-scan are deliberately fork-only divergences from upstream (see `docs/fork/DECISIONS.md#fork-identity-update-and-data-dir-behavior`) |
| `fork-agent-guide` | `880ee0a` (1 commit) | `ca319dc` | n/a (docs-only, no source patch) | Complete — adds `docs/fork/AGENT-WORKFLOW.md`, an `AGENTS.md` pointer, and a pointer-only `CLAUDE.md` | N/A — fork-internal agent guidance, not upstream-proposable |
| `fork-tpatch-bootstrap` | `24376a6` (1 commit) | `ca319dc` | `FEATURES.md` intentionally empty; this workspace tracks only customizations made from here forward | Complete — `tpatch doctor`/`tpatch status` verified clean after sanitizing the auto-detected local provider endpoint out of `.tpatch/config.yaml` | N/A — tooling bootstrap, not upstream-proposable |
| `classify-gitlab-range-anchors` | `958c815`, `d5cbbdd` (2 commits) | `4f60eda` | Tracked feature: `state: applied`, landing evidence exact at `d5cbbdd`; replay and preimage verification pass | Live-complete on GitLab 19.2.1: 62 focused tests plus backend/TUI/ReviewStore head-shift checks pass; valid 70-72 native ranges survive old-head or relocated-terminal stale shapes | Fork-only; upstream split TBD |
| `capture-provider-neutral-context-for-local-line-and-range` | `59faa80` (1 commit) | `c40f552` | Tracked feature: `state: applied`, landing evidence exact at `59faa80`; replay and preimage verification pass | Live-complete on Ubuntu 24.04/WSL2: line 42 moved to 47 through real TUI reload and persisted through CLI/durable-store inspection; 1,666 supported locked library tests pass | Fork-only for now; upstream split TBD |
| `wire-gitlab-durable-tui-publication` | `9330ee0`, `e6d6d80` (2 commits) | `5886486` | Tracked feature: `state: applied`, landing evidence exact at `e6d6d80`; replay and preimage verification pass | Live-complete on GitLab 19.2.1: real TUI root/reply/resolve/reopen, provider-ID reconciliation, restart persistence, duplicate-free retry, mock partial-failure resume, and 1,676 supported locked library tests pass | Fork-only for now; review-level multi-step publication remains on the legacy path |

The commit that added `docs/fork/DECISIONS.md`, `docs/wayfinder/`, and
`docs/handoff/CURRENT.md` carries no `Tpatch-Feature:` trailer and is
intentionally not listed as a row above — this table only lists commits with
a real, committed trailer, never an invented one.

## Reading this table

- "Commit range" is oldest-first within the feature's committed trailers on
  this branch's ancestry (`git log --grep 'Tpatch-Feature: <slug>$'`).
- "Base" is the parent of the range's first commit, i.e. the commit the
  feature branched from.
- Ranges for `provider-capabilities`, `gitea-forgejo-adapter`, and everything
  after them overlap in wall-clock time with earlier features' follow-up/audit
  commits interleaved on the same lineage; the ranges above are per-trailer,
  not strictly disjoint by commit position.

## How this list was produced

The historical rows (everything through `offline-release-candidate`) were
derived by hand from:

```
git log --all --format='%H|%ad|%s|%(trailers:key=Tpatch-Feature,valueonly)' --date=short \
  | awk -F'|' '$4!=""' | sort -t'|' -k4,4 -k2,2 | uniq
```

Re-run this against the branches you care about to refresh the list before
relying on it for anything other than a starting pointer; do not hand-edit
entries without re-deriving them from `git log`.

## Not represented here

Any `tpatch` feature that has not landed as a commit carrying a
`Tpatch-Feature:` trailer on a branch reachable from this repo's history is
not listed above. Do not add speculative rows for planned or in-progress
work; wait until it lands, then regenerate.

## Relationship to this repo's `.tpatch/` workspace

This repo's own `.tpatch/` workspace was bootstrapped by
`fork-tpatch-bootstrap` at `24376a6`; `classify-gitlab-range-anchors` and
`capture-provider-neutral-context-for-local-line-and-range` and
`wire-gitlab-durable-tui-publication` are complete tracked customizations.
`preserve-azure-native-anchors` is also landed, tracked, and live-proven
through the adapter, durable store, and real TUI.
See
[`../../.tpatch/README.md`](../../.tpatch/README.md) for why historical
trailers are not backfilled and for the lifecycle every new patch-bearing
customization follows (`tpatch add` → `analyze` → `define` → `explore` →
`implement` → `apply` → `record`/`test` → `verify` → `reconcile` → `land`).
