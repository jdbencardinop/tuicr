# Analysis: capture provider-neutral context for local anchors

## Observed behavior

- `AnchorContext`, `Anchor::line_with_context`,
  `Anchor::range_with_context`, and unique context relocation already model the
  required provider-neutral behavior. Zero matches become `Stale`, multiple
  matches become `Ambiguous`, and no nearest-line fallback exists.
- `ReviewSession::migrate_legacy_comments_to_threads` creates deterministic
  durable threads, but line and range anchors use context-free constructors.
  `App::mirror_new_comments_as_threads` calls that migration immediately after
  a TUI comment is saved, so the displayed diff and exact old/new side are
  available when context should be captured.
- `App::refresh_thread_anchors_after_head_advance` already fetches full file
  content and refreshes durable anchors, but ordinary local `:e` reloads do
  not call it.
- Local comment rendering remains backward-compatible and reads
  `FileReview::line_comments`, keyed by the original numeric terminal line.
  Refreshing only the durable thread would therefore move navigation/status
  state while leaving the visible legacy comment box on the old row.
- Existing rendering already derives a mirrored legacy comment's status from
  its durable thread, so leaving stale or ambiguous comments at their last
  known row produces an explicit typed label without inventing a destination.

## Existing seams

- `DiffFile` contains the exact displayed old/new line numbers and content
  needed to capture a bounded context window on either side without a provider
  call.
- `Thread::refresh_anchor_with_remap` and
  `PersistedThread::refresh_anchor_with_remap` are the canonical relocation
  entry points.
- `reload_diff_files` has both the replacement diff and the existing session,
  making it the local refresh integration point.
- Durable thread comment IDs preserve legacy comment IDs, allowing relocated
  current anchors to re-key only their own legacy shadows deterministically.

## Compatibility and risk

- Cold-loaded legacy sessions have no trustworthy historical context. They
  must remain context-free rather than capturing today's content at an old
  numeric position and pretending it is the original anchor.
- Context should be attached only to threads newly created by the current TUI
  save operation. Review-level and file-level anchors need no context.
- Line/range context must come from the selected diff side. Old-side deletion
  content must never be replaced with same-numbered new-side content.
- A range remains keyed in the legacy map by its inclusive terminal line.
  When a current durable range relocates, both its range metadata and map key
  must move together.
- Only uniquely relocated `Current` anchors may re-key legacy comments.
  `Stale` and `Ambiguous` comments remain visible at the last known row with
  their typed status.
- Closed threads stay frozen under the existing domain contract and therefore
  do not move their legacy shadows.
- A local reload may not have full trustworthy content for every side/source.
  Missing content must leave an anchor unchanged rather than synthesizing
  success; content that is available must use the existing exact relocation
  rules.

## Recommendation

Add a small domain API for attaching validated context to an existing
context-free line/range anchor. During TUI mirroring, snapshot existing thread
IDs, migrate, and attach bounded context only to the newly created threads
using exact side-aware lines from the displayed diff. Generalize the existing
App refresh helper for local reload, refresh context-bearing anchors against
updated file content, and then synchronize current durable line/range targets
back into their legacy shadows. Cover line 42 to 47, range relocation,
old/new-side capture, file-anchor stability, stale/ambiguous classification,
idempotency, and visible annotation movement.
