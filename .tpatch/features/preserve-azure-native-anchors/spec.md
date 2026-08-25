# Specification: preserve Azure DevOps native anchors

## Goal

Retain Azure DevOps iteration-relative anchor evidence through adapter
conversion and durable ReviewStore persistence without guessing relocation.

## Acceptance criteria

1. `list_review_threads` emits an opaque native-anchor object containing each
   present `threadContext` and `pullRequestThreadContext` exactly as modeled.
2. Absent provider context fields remain absent rather than becoming `null`.
3. Existing right-side precedence remains unchanged when Azure supplies both
   sides.
4. A selected-side multi-line span produces a validated normalized range.
   Single-line spans remain line anchors.
5. Reversed selected-side endpoints produce no normalized range and mark the
   thread outdated.
6. The opaque payload retains both sides, offsets, iteration IDs, change
   tracking ID, and tracking criteria.
7. Generic durable import stores the payload under `native_anchor`; repeat
   import merges by provider thread ID without duplication and preserves
   stale state.
8. Focused Azure adapter and durable-import tests, formatting, clippy, and
   the supported library suite pass.
9. A disposable Azure DevOps rerun proves adapter/TUI/ReviewStore behavior
   before the ticket closes.

## Non-goals

- Guessing the relocated line from Azure tracking criteria.
- Changing Azure's right-side precedence.
- Making draft pull requests votable.
- Adding private provider payloads to tracked evidence.
