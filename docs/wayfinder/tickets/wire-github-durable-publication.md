---
id: wire-github-durable-publication
title: Publish durable GitHub threads from the TUI
type: implementation
mode: autonomous
status: closed
owner: copilot
blocked_by: []
---

## Question

Wire GitHub `:submit comment` to the canonical durable publication
planner/executor so ReviewStore roots, replies, resolve/reopen transitions,
provider mappings, and partial-failure checkpoints behave as the offline
adapter contract claims.

## Completion

- The confirmation dialog previews the exact durable GitHub operation plan.
- Root, reply, resolve, and reopen operations publish through the real TUI.
- Provider mappings checkpoint after every successful mutation.
- Restart/retry sends only remaining operations and creates no duplicate.
- Legacy review bodies and inline comments retain their current behavior.
- Focused mock tests cover partial failure, concurrent session merge, and
  provider-ID re-import before the live disposable-repository rerun.

## Evidence

The 2026-08-25 live run created a durable range thread in ReviewStore, but
`:submit comment` reported `Nothing to submit`; no GitHub mapping or provider
thread was created. Sanitized evidence:
`../../../../artifacts/validation/2026-08-25-wsl-github/`.

The 2026-08-27 rerun passed range-root, reply, resolve, reopen, checkpoint,
same-process retry, restart retry, and duplicate-prevention checks. Sanitized
evidence:
`../../../../artifacts/validation/2026-08-27-wsl-github-durable-publication/`.

## Resolution

GitHub non-draft comment submission now uses the same durable plan,
checkpointed executor, and provider-ID reconciliation path as GitLab while
GitHub draft reviews and legacy-only sessions retain their existing paths.
The live rerun also corrected GitHub review-thread discovery to scan the
paginated pull-request `reviewThreads` connection and made checkpoint mapping
keys provider-aware. The disposable repository was deleted after the final
duplicate-free restart and reopen checks.
