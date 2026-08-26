---
id: validate-live-provider-parity
title: Validate live GitHub/GitLab/Azure DevOps provider parity
type: prototype
mode: HITL
status: blocked
owner: copilot
blocked_by: [provision-provider-sandboxes, classify-gitlab-range-anchors, wire-gitlab-durable-publication, wire-github-durable-publication, fix-github-head-refresh]
---

## Question

Prove — against real, approved provider targets — that the durable thread
model and each adapter's mutation behavior match what the offline/mock
contract tests already claim, without regressing existing GitHub/GitLab
behavior.

## Completion

- **GitHub/GitLab:** draft/batch, comment, approve, request-changes
  (or its emulation), replies, resolution, new-head sync, and duplicate
  prevention pass in approved disposable targets.
- **Azure DevOps:** read, dry-run, comment, approve/wait/reject/reset,
  reply/status, anchor shift, idempotency, and cleanup pass in an approved
  sandbox.

## Status

GitLab is live-tested. GitHub has a completed one-identity live run with two
implementation blockers. Azure DevOps thread, stale-anchor, durable-store,
and TUI behavior are live-tested; non-draft vote behavior remains blocked by
reviewer-notification policies:

- GitHub/GitLab offline/mock parity is complete (durable create/reply/resolve,
  REST/GraphQL ID mapping, partial-resume/idempotency, head-update handling,
  error/redaction fixtures, shared dry-run planning) — `tpatch` feature
  `github-gitlab-thread-parity-offline`, see `docs/fork/PATCHES.md`.
- Azure DevOps adapter is offline/mock-verified against Microsoft
  Learn-cited fixtures and mock HTTP contract tests (grew 52 → 60 passing
  tests across two audit-fix rounds) — `tpatch` feature
  `azure-devops-adapter-offline`, see `docs/fork/PATCHES.md`. An `#[ignore]`d
  live-sandbox harness (`src/forge/azure/live_tests.rs`, env-var gated) is
  committed and has been run against approved disposable targets.

### Live GitHub result — 2026-08-25

The proposed two-identity public sandbox was unavailable because the second
identity is an Enterprise Managed User. The user approved a one-identity run
against a disposable public repository and draft pull request containing only
the synthetic fixture.

Observed passes:

- The real WSL TUI rendered the initial 14-file, 1,000-line pull-request diff.
- One legacy review body and one line-42 inline comment published in a single
  `COMMENTED` review.
- Repeated `:submit comment` was a local no-op and created no duplicate.
- After the fixture head advanced, GitHub relocated the inline comment from
  line 42 to line 47 and retained its original line/commit metadata.
- The exact draft pull request was closed and the exact repository was
  deleted after granting the CLI credential only the teardown scope required
  for repository deletion.

Observed failures and evidence gaps:

- A durable ReviewStore range thread was ignored by `:submit comment`, which
  reported `Nothing to submit`; no GitHub mapping or remote thread was
  created.
- After the head update, both reload and restart rendered an empty diff and
  omitted the provider-relocated inline comment even though GitHub still
  returned a valid 14-file diff and the comment at line 47.
- Durable reply/status/resume behavior could not be exercised because the
  durable root was not publishable through the TUI.
- Independent approve/request-changes behavior remains untested because only
  one standard GitHub identity was available.

Sanitized evidence:
`artifacts/validation/2026-08-25-wsl-github/`.

### Live Azure DevOps result — 2026-08-25

An approved single-identity disposable draft PR changed only a 200-line
synthetic file in the approved repository directory. It had zero reviewers
throughout.

Observed passes:

- Adapter-driven inline thread creation, reply, fixed/active status
  transitions, and read round-trip passed.
- A live Connection Data version failure was fixed at `50c4580`; the rerun
  resolved the viewer identity and reached the reviewer vote endpoint.
- After five synthetic lines were inserted before the anchor, the adapter
  classified the original thread as outdated.
- Cleanup deleted both temporary threads/comments, abandoned the draft PR,
  and deleted the exact temporary branch. Initial and final votes were zero.

Observed failures and gaps:

- Azure DevOps forbids voting on draft PRs, so approve/wait/reject/reset
  remains untested.
- Azure returned thread and iteration context after the head update, but the
  adapter dropped it from `provider_native_anchor`.
- TUI/ReviewStore publication, checkpoint resume, duplicate prevention, and
  two-PR pagination were not run after the native-anchor failure.

Sanitized evidence:
`artifacts/validation/2026-08-25-wsl-azure-devops/`.

### Azure native-anchor rerun — 2026-08-25

The tracked `preserve-azure-native-anchors` implementation at `c3d6dde`
passed a second approved disposable draft-PR run. Azure's current and shifted
thread payloads retained `threadContext` and
`pullRequestThreadContext`; the shifted thread remained explicitly outdated.
The native payload survived ReviewStore save/reload, and repeat import merged
by provider thread ID without duplication. Root/reply and fixed/active
transitions also passed. Cleanup deleted the synthetic comments, abandoned
the PR, and deleted the exact branch; reviewer count and vote remained zero.

The TUI could open the abandoned PR from a sparse matching checkout, proving
the Azure local-diff prerequisite, but cleanup had already deleted the
synthetic comment. Therefore TUI rendering is not claimed and
at that checkpoint `preserve-azure-native-anchors` remained open for one
TUI-before-cleanup rerun.
Draft vote parity and two-PR pagination also remain open. Sanitized evidence:
`artifacts/validation/2026-08-25-wsl-azure-devops-native-anchor/`.

### Azure TUI native-anchor rerun — 2026-08-26

An approved synthetic-only draft PR with zero reviewers completed the
previously missing TUI-before-cleanup check. The current adapter/ReviewStore
phase passed, then five synthetic lines were inserted before the anchor. The
shifted adapter payload remained outdated with native iteration context,
ReviewStore save/reload and repeat import remained duplicate-free, and the
real TUI rendered the root and reply at line 60 with
`outdated · locally stale`. The exact comments were deleted, the PR
abandoned, and the branch deleted; reviewer count and vote remained zero.

A separate synthetic non-draft attempt was stopped before comments or votes
because Azure policy automatically added six reviewers. It was immediately
abandoned and its branch deleted with vote zero. The linked production PR was
never mutated. This establishes that vote parity needs a policy-free Azure
DevOps sandbox rather than another enterprise-repository PR. Sanitized
evidence:
`artifacts/validation/2026-08-26-wsl-azure-devops-tui-anchor/`.

### Live GitLab 19.2.1 result — 2026-08-11

Approved disposable target:
`gitlab/gitlab-ce:19.2.1-ce.0` at manifest digest
`sha256:2777b4a990a05a40947437d3c8e0e03347974d8998663d266064cebaff00ba87`,
remote-loopback only through SSH/Azure Bastion, public synthetic fixture,
one-day PATs, and exact teardown.

Observed passes:

- Tuicr opened the self-hosted MR and rendered the 1,000-line diff.
- TUI `Comment` submitted one review-level summary and one inline line-42
  discussion; a second submit reported no local drafts and created no
  duplicate.
- TUI `Approve` produced GraphQL reviewer state `APPROVED`.
- TUI `Request changes` with a summary produced reviewer state
  `REQUESTED_CHANGES`; an empty request-changes attempt is rejected locally as
  "nothing to submit", unlike empty approval.
- A temporary external harness calling the real `GitLabGlabBackend` passed
  create range thread (70-72), reply, resolve, reopen, final resolve, and
  list-roundtrip. GitLab API state independently confirmed two notes, the
  range, and final resolution.
- After force-updating the fixture head, the legacy single-line discussion
  relocated from 42 to 47 and Tuicr rendered it on the same semantic line.
- Container, named volumes, local forward, and all credential/config state
  were removed; no GitLab payload or credential remains on disk outside
  gitignored raw evidence.

Observed gaps:

- TUI submission explicitly keeps durable thread roots/replies/resolution
  local-only. The backend methods work live, but the TUI publication path is
  not wired to execute them and persisted no provider mapping —
  [diffreviewtui#3](https://github.com/jdbencardinop/diffreviewtui/issues/3).
- A full self-hosted URL containing custom port `:8929` fails through `glab`
  (`--hostname` rejects ports and MR repository URLs force HTTPS). The working
  configuration uses a portless logical GitLab hostname mapped by `glab` to
  the port-bearing HTTP API host —
  [diffreviewtui#2](https://github.com/jdbencardinop/diffreviewtui/issues/2).

### GitLab stale-range rerun — 2026-08-20

The approved loopback-only GitLab 19.2.1 fixture was recreated with the public
1,000-line synthetic repository and one-day credentials. The original pinned
implementation classified the shifted range stale but discarded its range:
GitLab rewrote the native `head_sha` to the current MR head, relocated
`new_line` from 72 to 77, and retained `line_range` 70-72.

Commit `d5cbbdd` now treats that provider-terminal mismatch as stale evidence
without guessing relocation. The committed harness, default/all/reloaded TUI
views, and durable `ReviewStore` all passed with the preserved stale range at
70-72. Exact container, volume, credential, build-state, forward, and tunnel
teardown passed. Durable TUI publication remains the only GitLab integration
gap.

## Unblock condition

Close `wire-github-durable-publication` and `fix-github-head-refresh`, then
rerun the disposable GitHub lifecycle. Azure vote parity additionally
requires a policy-free non-draft disposable PR. Record each remaining
pass/fail delta directly in this ticket.
