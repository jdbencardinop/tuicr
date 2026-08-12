# Analysis: classify GitLab range anchors

## Observed behavior

- `GitLabGlabBackend::list_review_threads` already receives
  `PullRequestDetails`, including the current MR `head_sha`.
- `GlabNotePosition` parses the position `head_sha`, but
  `GlabDiscussion::into_review_thread` ignores it and hardcodes
  `RemoteReviewThread.is_outdated = false`.
- GitLab range positions include a same-side `line_range.start` and
  `line_range.end`, but the read model does not deserialize or retain them.
- `RemoteReviewThread` carries only one line and no opaque provider-native
  anchor. `thread_from_remote` therefore imports a GitLab range as a single
  line and writes only generic ID/path/line/status fields into
  `provider_mappings`.
- `thread_from_remote` stores `is_outdated` in provider metadata but leaves the
  durable `Thread` and `Anchor` in `Open`/`Current`, which can make a stale
  imported thread appear actionable to durable planning.
- `merge_remote_thread_into_existing` deliberately keeps the existing anchor.
  A current thread that becomes provider-outdated on a later fetch would
  otherwise acquire `ThreadStatus::Stale` from the fresh copy while retaining
  `AnchorState::Current`; resolving and reopening it would silently return it
  to open.

The 2026-08-11 disposable GitLab 19.2.1 run observed the practical result: a
legacy single-line discussion moved from 42 to 47, while the durable range
remained at 70-72 and was imported as not outdated after the MR head advanced.

## Existing seams

- `GlabDiscussion::into_review_thread` is the provider boundary that already
  selects path, side, and line.
- `ForgeBackend::list_review_threads` has the current MR head and needs no new
  request.
- `RemoteReviewThread` is the provider-neutral transfer object used by every
  adapter and by durable import.
- `AnchorTarget::Range`, `AnchorState::Stale`, and `ThreadStatus::Stale`
  already model the required normalized result.
- `PersistedThread.provider_mappings` is explicitly intended to preserve
  opaque provider-native anchor data.

## Compatibility and risk

- Additive optional fields on `RemoteReviewThread` can remain absent from
  serialized output for providers that do not populate them.
- A validated `RemoteReviewRange` type must reject reversed endpoints during
  construction and deserialization, rather than normalizing or guessing.
- GitLab range endpoints must be ordered and compatible with the selected
  position side. GitLab legitimately emits `type: null` for unchanged/context
  endpoints with both old/new lines; those use the line for the selected side
  and must not be rejected. Explicit mixed old/new sides, unknown endpoint
  types, missing selected-side lines, reversed ranges, or terminal-line
  mismatches are invalid. Invalid provider shapes remain visible only as
  outdated and retain their opaque native position for diagnosis.
- A missing/empty position head or current MR head is insufficient evidence of
  staleness; classify only a non-empty mismatch.
- A resolved-and-outdated thread should retain a stale anchor while its thread
  status remains resolved; reopening must return it to stale.
- Repeat import may promote a current anchor to provider-stale, but a later
  provider fetch must never reset a locally stale/ambiguous anchor to current
  or overwrite an existing ambiguous signal.
- Context-less imported anchors participate in head-advance refresh. That
  refresh must preserve an existing stale/ambiguous state when no exact
  provider remap or relocation context exists; otherwise a transient refetch
  failure can resurrect a provider-stale thread as open.
- All non-GitLab producers and test fixtures must explicitly carry no range or
  native anchor, avoiding behavior changes.

## Recommendation

Extend the remote transfer object with a validated optional range and optional
opaque native anchor. Capture the original `serde_json::Value` for GitLab's
position before typed validation so unmodeled provider fields are not lost.
Pass the current MR head into conversion, retain valid range/native data,
classify version mismatches or malformed ranges as outdated, and make initial
and repeated durable imports construct range anchors plus monotonic
provider-stale anchor state. Cover model conversion, backend propagation,
initial/repeat durable import, resolved/stale lifecycle, serialization
compatibility, context-line endpoints, and malformed-range behavior. No live
provider call is required for this offline implementation; the Wayfinder
ticket remains open for the disposable GitLab rerun.
