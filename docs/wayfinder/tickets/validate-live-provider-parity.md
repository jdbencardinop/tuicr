---
id: validate-live-provider-parity
title: Validate live GitHub/GitLab/Azure DevOps provider parity
type: prototype
mode: HITL
status: blocked
owner: copilot
blocked_by: [provision-provider-sandboxes]
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

Both halves are **offline/mock-complete, live-blocked**:

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
- **No live GitHub, GitLab, or Azure DevOps target has been contacted or
  mutated.** This is the single remaining evidence gap for all three
  providers.

## Unblock condition

`provision-provider-sandboxes` must close first. Then run the existing
offline-proven operations against the real, approved targets and record the
pass/fail results and any behavioral deltas directly in this ticket's
Resolution section.
