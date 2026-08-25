# Exploration: wire GitLab durable TUI publication

## Primary changes

### `src/forge/publish.rs`

- Add checkpointed execution while retaining the existing `execute_plan`
  wrapper for current callers and tests.
- Invoke the checkpoint only after an operation has updated its in-memory
  provider mapping and before advancing.
- Return the review response and preserve failed/remaining operation details.

### `src/app/submit.rs`

- Build a GitLab-only durable publication clone after legacy mapping.
- Retain mapped threads for replies/status and validated thread-native roots.
- Remove non-selected legacy comments from grouped threads at comment
  granularity.
- Assert every selected inline comment has exactly one durable operation.
- Run GitLab comment submits containing only durable thread operations through
  the durable executor; keep review bodies, review actions, drafts, resolver
  work, hidden grouped roots, and other providers on the legacy path.
- Checkpoint successful provider mappings atomically and reconcile results on
  the main thread without replacing thread content.

### `src/app/mod.rs`

- Extend submit and in-flight state with an optional durable plan and the
  publication-session snapshot needed for exact execution.
- Extend background events with a durable publication result carrying the
  report and final provider mappings.

### `src/app/session.rs`

- Merge provider mappings field-wise for an existing thread even when another
  local field changed, so an external publication checkpoint cannot be lost.
- Add focused concurrent-edit/checkpoint regression coverage.

### `src/ui/submit_modals.rs`

- Replace the GitLab local-only warning with counts for each durable operation
  outcome.
- Surface emulation and skipped/partial behavior before confirmation.

### `src/model/thread.rs` / `src/model/thread_store.rs`

- Reuse deterministic legacy thread identity to distinguish historical mirror
  threads from genuinely thread-native roots.
- Add only the minimal comment-filtering API required to remove resolver-
  excluded grouped replies from a publication clone.

## Tests

- Preflight plan covers every selected inline legacy comment exactly once.
- Moved-to-summary and omitted grouped replies are absent.
- Historical mirror without a current legacy shadow is excluded.
- Mapped imported threads remain available for reply/resolve/reopen.
- Root mapping, reply ledger, and resolution mapping checkpoint after each
  success.
- A forced failure after a successful prefix persists that prefix.
- Restart and retry emit only remaining operations.
- Complete retry emits no duplicate thread operation.
- Concurrent local reply plus external mapping checkpoint preserves both.
- GitLab draft and GitHub submit continue using legacy create-review payloads.
- Modal summary reports planned, emulated, and skipped outcomes.

## Invariants

- Durable threads remain canonical; legacy comments remain rendering and API
  compatibility shadows.
- One planner and one executor own publication semantics.
- No provider mutation occurs before confirmation.
- No successful provider mutation is followed by another mutation until its
  mapping checkpoint succeeds.
- Background results merge mappings only into still-existing current threads.
- Draft privacy is never weakened.
- Provider credentials and payloads are never persisted.

## Validation

Run formatting, focused submit/planner/executor/persistence tests, all-target
clippy with warnings denied, and the supported locked library suite. Complete
the tracked `tpatch` lifecycle, then run the approved disposable GitLab 19.2.1
TUI lifecycle using isolated configuration and retain raw output only under
ignored `artifacts/raw/`.
