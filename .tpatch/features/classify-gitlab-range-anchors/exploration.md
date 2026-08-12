# Exploration: classify GitLab range anchors

## Primary changes

### `src/forge/remote_comments.rs`

- Add `RemoteReviewRange`, with private `start`/`end`, validated construction,
  accessors, serialization, and validating deserialization.
- Add optional `range` and `provider_native_anchor` fields to
  `RemoteReviewThread`, using serde defaults and omitting absent values.
- Unit-test range construction/deserialization and unchanged serialization
  when optional fields are absent.

### `src/forge/gitlab/models.rs`

- Make `GlabNotePosition` serializable and add optional `line_range`.
- Add typed start/end endpoint DTOs (`type`, old/new line, line code), treating
  `type: null` as a context endpoint that must provide both sides.
- Preserve the original raw position `serde_json::Value` separately from the
  typed validation view, including fields the typed model does not know.
- Change `GlabDiscussion::into_review_thread` to receive the current MR head.
- Compute staleness only from non-empty position/current head mismatch.
- Validate range side/order/end agreement. Keep valid normalized ranges;
  classify malformed ranges outdated without guessing.
- Store the full serialized GitLab position as the opaque native anchor.
- Update existing conversion tests and add current/stale/missing/malformed
  range cases.

### `src/forge/gitlab/glab.rs`

- Pass `pr.head_sha` to every discussion conversion in
  `list_review_threads`.
- Add a runner-backed test proving current and old-head discussions are
  classified from the `PullRequestDetails` head.

### `src/model/thread.rs`

- Add explicit provider-stale lifecycle methods that set anchor state and
  open-family thread status to stale while preserving closed-state rules and
  existing stale/ambiguous signals.
- Make context-less refresh return the anchor's existing stale/ambiguous state
  rather than synthesizing `Current`; an exact provider remap remains the only
  context-less path back to current.
- Test stale marking and resolved/reopen behavior.

### `src/model/thread_store.rs`

- Build a durable range anchor when `RemoteReviewThread.range` is present.
- Apply provider-outdated state before resolution.
- Merge the optional native anchor and normalized range into the provider
  mapping without changing the existing ID/path/line/status keys.
- On repeat import, monotonically promote a still-current existing anchor to
  provider-stale when the fresh mapping is outdated; never reset or overwrite
  stale/ambiguous local relocation state.
- Test range target, opaque mapping round-trip, first/repeat stale import,
  ambiguous preservation, and resolved-stale reopen behavior.

## Mechanical compatibility updates

Every existing `RemoteReviewThread` literal must set `range: None` and
`provider_native_anchor: None` unless it is the new GitLab range fixture. The
affected production/test files are discoverable with:

```text
rg "RemoteReviewThread \\{" src
```

Current matches span GitHub, Azure DevOps, Gitea/Forgejo, publish/export,
submit-modal, unified/side-by-side UI, row-height, and app tests. These are
mechanical additions only; no provider behavior outside GitLab changes.

## Invariants

- No nearest-line or endpoint normalization.
- A native position is retained even when malformed or old-head.
- A missing SHA is unknown, not stale.
- GitLab `type: null` context endpoints are valid on the selected side when
  both old/new lines are present.
- Provider-stale state survives head refresh even if the subsequent remote
  refetch fails.
- Individual/general notes remain non-positional and current.
- Existing legacy GitLab single-line handling remains intact.
- No network calls are added.

## Validation

Run formatting, focused tests for:

```text
cargo test --lib forge::gitlab
cargo test --lib model::thread
cargo test --lib model::thread_store
```

Then clippy and the full supported library suite. Record the working-tree patch
before committing, run `tpatch test`/`verify`, and use `tpatch land` or a
single commit with the required feature/checksum/base and Copilot trailers.
The live GitLab rerun remains a separate HITL step.
