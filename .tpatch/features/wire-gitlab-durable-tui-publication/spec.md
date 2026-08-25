# Specification: wire GitLab durable TUI publication

## Goal

Publish GitLab durable roots, replies, and thread status transitions from the
existing TUI confirmation flow with an exact operation preview and
checkpointed provider mappings that make partial failures safely resumable.

## Acceptance criteria

1. GitLab `:submit comment` preflight with no review-body or resolver work
   builds its preview with
   `plan_publication` and confirmation displays planned, emulated, unsupported,
   stale, conflict, and invalid operation counts.
2. GitLab durable roots, replies, resolve, and reopen
   execute only through `execute_plan`.
3. Every successful thread operation atomically persists the complete
   provider-mapping map, including root and reply ledgers, before the next
   provider mutation begins.
4. A failure stops execution, reports the failed and remaining operations, and
   leaves every successful prefix mapping available after process restart.
5. Replanning after a partial failure omits completed roots, replies, and
   status transitions and resumes only remaining work.
6. Replanning after complete success produces no duplicate thread operation.
7. Legacy comments selected as inline are each represented exactly once by a
   durable root or reply. Comments moved to summary or omitted are absent from
   durable operations, including when several comments share one anchor.
8. Existing orphaned legacy mirror threads are not mistaken for new
   thread-native roots and republished.
9. Successful root/reply checkpoints transition their corresponding legacy
   comments out of `LocalDraft`; skipped or unattempted comments remain
   retryable.
10. Provider mappings from background checkpoints merge field-wise into the
    current session without discarding concurrent local thread edits or
    resurrecting deleted threads.
11. GitLab drafts, approvals, request-changes, review-level bodies, resolver
    work, hidden grouped roots, and every non-GitLab provider retain the
    existing legacy submit behavior.
12. Skipped operations are stated honestly in confirmation and completion
    messages.
13. Focused mock tests cover root, grouped reply, resolve, reopen, partial
    failure, restart/resume, completed no-op retry, resolver exclusion,
    orphaned mirror exclusion, and unchanged GitHub behavior.
14. A disposable GitLab 19.2.1 TUI rerun verifies root, reply, resolve, reopen,
    restart persistence, and duplicate-free retry.

## Implementation plan

1. Add publication-plan data and outcome summaries to submit state and modal
   rendering.
2. Add a deterministic helper that creates the exact GitLab publication
   session from preflight-selected legacy IDs and durable thread provenance.
3. Extend `execute_plan` with a checkpoint callback and captured review
   response while preserving the existing convenience entry point.
4. Add atomic persistence helpers that merge only a successful thread's
   provider mappings and add field-wise mapping reconciliation to normal
   session merge.
5. Add a GitLab non-draft background submit branch that fetches PR details,
   executes the stored plan, checkpoints each success, and returns the report
   plus final mapping state.
6. Reconcile result mappings and per-comment lifecycle on the main thread,
   report partial results, and refetch remote threads.
7. Add focused tests, run offline gates, then run and sanitize the approved
   disposable GitLab validation.

## Non-goals

- Changing GitHub, Azure DevOps, Gitea, or Forgejo publication behavior.
- Replacing the durable planner, executor, or ReviewStore.
- Adding GitLab pending-review or resumable review-level action semantics.
- Implementing optional known-failure issues #2 or #3.
- Posting to any provider outside the approved disposable GitLab sandbox.

## Accepted tradeoff

GitLab draft submissions remain on the legacy draft-notes path because
standalone durable thread creation is immediately visible and cannot preserve
draft privacy.
