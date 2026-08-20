---
id: validate-live-provider-parity
title: Validate live GitHub/GitLab/Azure DevOps provider parity
type: prototype
mode: HITL
status: blocked
owner: copilot
blocked_by: [provision-provider-sandboxes, classify-gitlab-range-anchors, wire-gitlab-durable-publication]
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

GitLab is now **live-tested with one integration gap**. GitHub and Azure
DevOps remain live-blocked:

- GitHub/GitLab offline/mock parity is complete (durable create/reply/resolve,
  REST/GraphQL ID mapping, partial-resume/idempotency, head-update handling,
  error/redaction fixtures, shared dry-run planning) — `tpatch` feature
  `github-gitlab-thread-parity-offline`, see `docs/fork/PATCHES.md`.
- Azure DevOps adapter is offline/mock-verified against Microsoft
  Learn-cited fixtures and mock HTTP contract tests (grew 52 → 60 passing
  tests across two audit-fix rounds) — `tpatch` feature
  `azure-devops-adapter-offline`, see `docs/fork/PATCHES.md`. An `#[ignore]`d
  live-sandbox harness (`src/forge/azure/live_tests.rs`, env-var gated) is
  committed and compiles but has never been run.
- **No live GitHub or Azure DevOps target has been mutated.**

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

Close `wire-gitlab-durable-publication`, provision and validate the approved
GitHub/Azure DevOps targets, then re-run the GitLab publication lifecycle.
Record each remaining pass/fail delta directly in this ticket's Resolution
section.
