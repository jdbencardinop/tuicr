# Fork Patch Index

Historical index of the `tpatch`-tracked features that make up this fork,
derived from committed `Tpatch-Feature:` trailers in `git log`. This is the
durable record.

## Honesty note on `.tpatch/` metadata

`tpatch`'s own per-feature state (`.tpatch/features/<slug>/status.json`,
`FEATURES.md`) lives under a gitignored `.tpatch/` directory
(`.git/info/exclude`), so it is **local, uncommitted, and worktree-specific**
— it does not travel with `git clone`/`git log` and can vanish if a worktree
is pruned. This table treats the committed `Tpatch-Feature:` trailer as the
only guaranteed-durable identifier, and reports local `.tpatch` state only
where it was directly inspected in a still-present sibling worktree. Where it
was not (re)inspected, this is stated explicitly rather than assumed.

## Source vs. reconciled commits

Every feature below was built directly in this fork's own commit lineage
**except Azure DevOps**, which was implemented in a separate sibling
repository/worktree (`azure-adapter-offline`, fetched here as remote
`azure-src`) and later re-applied ("reconciled") onto this lineage as new
commits with matching messages but different hashes. Both ranges are listed
for that feature; the source range is not reachable from this branch's
history, only the reconciled range is.

## Index

| Tpatch-Feature slug | Commit range (this lineage) | Base | Local `.tpatch` state (uncommitted, best-effort) | Validation state | Upstream target |
|---|---|---|---|---|---|
| `expose-review-comment-authors` | `6ea2048` (1 commit) | `f92502b` (upstream branch point) | not (re)inspected this pass | Implemented, targeted tests pass, verified via a 5-insertion/1-deletion diff | **Ready to propose** — see `docs/wayfinder/tickets/upstream-and-reconcile.md` and research workspace `docs/follow-on-map/tickets/01-upstream-author-json.md` |
| `editable-provider-neutral-threads` | `a65737c..60115b0` (5 commits) | `fb7d430` | not (re)inspected this pass | Implemented against frozen contract fixtures (migration, replies, resolution, authorship, provider mappings, cross-head identity, anchor states) | Fork-only for now; upstream split TBD |
| `provider-capabilities` | `9acea98..b61bdbb` (4 commits) | `60115b0` | not (re)inspected this pass | Implemented; registry/profiles/dry-run planner verified against fake provider profiles | Fork-only for now; upstream split TBD |
| `gitea-forgejo-adapter` | `795f2bc..7a61137` (4 commits) | `b61bdbb` | Inspected in sibling worktree `gitea-forgejo-adapter`: `state: applied`, last `verify` passed | Live-verified against disposable local **Gitea 1.24** and **Forgejo 16**; reply/resolve/edit routes confirmed unsupported (405 on stable Swagger) | Not proposed; new adapter, candidate after thread contract stabilizes |
| `azure-devops-adapter-offline` | **Source** (sibling repo, not on this branch): `db10560..50eaad5` (4 commits, remote `azure-src`, branch `azure-adapter-offline`). **Reconciled onto this lineage**: `7ee96d2..7078b82` (4 commits, same messages/order, different hashes) | `7a61137` | Inspected in sibling worktree `azure-adapter-offline`: `state: applied` (local/uncommitted; disappears if that worktree is removed) | Offline/mock-verified only (contract tests against Microsoft Learn-cited fixtures, grew 52→60 across audit rounds); **no live Azure DevOps org contacted** | Not proposed; blocked on live sandbox (`docs/wayfinder/tickets/provision-provider-sandboxes.md`) before upstream discussion is meaningful |
| `tui-thread-integration` | `732745c..9a965a2` (5 commits) | `7a61137` | not (re)inspected this pass | Implemented; canonical durable-thread TUI rendering, remote-overlay, reply-splice ordering, race-condition fix | Fork-only; TUI rendering is fork-specific by nature |
| `github-gitlab-thread-parity-offline` | `cc38057..7b086df` (3 commits) | `9a965a2` | not (re)inspected this pass | Offline/mock parity complete (durable create/reply/resolve, REST/GraphQL ID mapping, resume/idempotency, head-update handling) | N/A — parity work on existing adapters, not a new upstream-proposable feature; **live GitHub/GitLab mutation validation still blocked**, see `docs/wayfinder/tickets/validate-live-provider-parity.md` |
| `offline-integration` | `1838181..d8b1f63` (2 commits) | `7078b82` | not (re)inspected this pass | Complete — integrates the Azure reconciliation and the five-provider offline source into one lineage | N/A — integration glue, not an upstream-proposable feature |
| `offline-release-candidate` | `0ef4dcf..ca319dc` (5 commits) | `d8b1f63` | not (re)inspected this pass | `OFFLINE_VALIDATED_ONLY` — see `retrospectives/offline-release-acceptance.md` in the research workspace | **Never upstream** — fork identity, disabled self-update, isolated data dir, and packaging secret-scan are deliberately fork-only divergences from upstream (see `docs/fork/DECISIONS.md#fork-identity-update-and-data-dir-behavior`) |

## Reading this table

- "Commit range" is oldest-first within the feature's committed trailers on
  this branch's ancestry (`git log --grep 'Tpatch-Feature: <slug>$'`).
- "Base" is the parent of the range's first commit, i.e. the commit the
  feature branched from.
- Ranges for `provider-capabilities`, `gitea-forgejo-adapter`, and everything
  after them overlap in wall-clock time with earlier features' follow-up/audit
  commits interleaved on the same lineage; the ranges above are per-trailer,
  not strictly disjoint by commit position.
