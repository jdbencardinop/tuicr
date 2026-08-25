# Analysis: wire GitLab durable TUI publication

## Observed behavior

- `forge::dryrun::plan_publication` already produces capability-aware
  create-thread, reply, resolve, reopen, dismiss, and review operations.
- `forge::publish::execute_plan` already executes those operations in order
  and updates provider mappings, root-comment IDs, reply ledgers, and
  resolution snapshots in memory.
- The TUI `:submit` path still sends only legacy comments through
  `create_review`; durable-only replies and status transitions are explicitly
  reported as local-only.
- A successful executor prefix is not written to disk until its caller saves
  the returned session. A crash or later failure can therefore lose the
  provider IDs required to make a retry duplicate-safe.
- Legacy comments are compatibility shadows of durable thread comments.
  Multiple comments at one anchor may become one durable root plus replies,
  so publication selection must operate at comment granularity.

## Existing seams

- `SubmitState` owns preflight data and the confirmation renderer.
- `plan_publication` is the single publication planner and must remain the
  source of the operation preview.
- `execute_plan` is the single mutation executor and must remain the source of
  provider calls and mapping updates.
- `save_session_by_identity` provides locked, atomic session updates suitable
  for checkpointing a successful operation from the background worker.
- Provider mappings are isolated on `PersistedThread`, including namespaced
  root and reply ledgers that survive remote re-import.

## Compatibility and risk

- GitHub and other provider submit behavior must remain unchanged while this
  GitLab-specific ticket is validated.
- GitLab drafts currently use the legacy draft-notes path. Durable standalone
  thread creation would publish immediately, so drafts must remain on that
  legacy path.
- Existing sessions can contain orphaned legacy mirror threads after submitted
  comments were pruned. Absence of a current legacy shadow is not sufficient
  evidence that a root is thread-native.
- Resolver choices must remove moved or omitted legacy comments from the
  publication clone, including grouped durable replies.
- A successful remote operation followed by a later failure must checkpoint
  the complete provider-mapping map before advancing.
- Main-thread reconciliation must merge only provider mappings into current
  threads; replacing a thread from a background snapshot could discard edits
  or resurrect a deletion.
- Provider mappings written by a checkpoint must win field-wise during normal
  three-way session merges, even when another local thread field also changed.

## Recommendation

Use the durable path for GitLab `:submit comment` when the operation contains
only durable roots, replies, and status transitions. Review bodies and
multi-step review actions remain on the legacy path until they can be split
into independently checkpointed provider mutations. Build a filtered
publication-session clone whose legacy comments exactly match preflight
selection while mapped/imported and genuinely thread-native threads remain
available for replies and status transitions. Store the resulting
`DryRunPlan` in `SubmitState`, render all outcome counts, and execute it in the
background through `execute_plan`.

Extend the executor with an operation checkpoint hook. The TUI hook atomically
merges the successful thread's complete provider mappings into persisted
storage after every mutation. Return the updated mappings and publish report
to the main thread, merge mappings into current state, transition only legacy
comments whose thread operations succeeded, and preserve remaining activity
for a smaller retry plan.
