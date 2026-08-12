# Specification: classify GitLab range anchors

## Goal

Make GitLab discussion imports honest across MR head changes: retain valid
same-side ranges and opaque provider-native position data, classify known
version mismatches as outdated, and propagate that state into durable review
threads without guessing relocation.

## Acceptance criteria

1. `RemoteReviewThread` can optionally carry:
   - a validated inclusive `RemoteReviewRange`; and
   - an opaque provider-native anchor JSON value.
   Both fields are backward-compatible and omitted from serialized output when
   absent.
2. `RemoteReviewRange` rejects `end < start` during construction and
   deserialization. It never swaps or normalizes endpoints.
3. `GlabDiscussion::into_review_thread` accepts the current MR head:
   - equal non-empty position/current heads remain current;
   - different non-empty heads are outdated;
   - missing or empty values do not invent a stale signal.
4. A GitLab `line_range` is retained only when:
   - both endpoints are compatible with the selected old/new position side;
   - a `type: null` unchanged/context endpoint supplies both old/new lines and
     contributes the line for the selected side;
   - `start <= end`;
   - the range end agrees with the position's selected line.
   Explicit mixed sides, unknown endpoint types, missing selected-side lines,
   reversed endpoints, and terminal mismatches are malformed. A malformed
   range is not collapsed or normalized: the thread is marked outdated, its
   normalized range is absent, and the opaque native position is retained.
5. `GitLabGlabBackend::list_review_threads` passes `PullRequestDetails.head_sha`
   into every discussion conversion without another network request.
6. Durable remote import:
   - constructs `AnchorTarget::Range` for a retained range;
   - records range and opaque native-anchor data in the provider mapping;
   - marks provider-outdated anchors/threads stale;
   - preserves resolved-over-stale status while retaining stale anchor state,
     so reopening returns to stale.
   Repeat import of a previously current thread may monotonically mark its
   current anchor provider-stale, but never resets an existing local
   stale/ambiguous anchor or replaces ambiguous with stale.
   A later context-less head refresh also preserves that state unless an exact
   provider remap is supplied.
7. Existing GitHub, Azure DevOps, Gitea/Forgejo, rendering, export, and test
   fixtures carry no range/native anchor unless their adapter supplies one;
   their behavior and serialized output remain unchanged.
8. Regression tests cover current and old-head GitLab ranges, missing SHA,
   valid context-line (`type: null`) endpoints, malformed
   mixed-side/reversed/mismatched-end ranges, backend propagation,
   range/native durable mapping, first import, current-then-outdated repeat
   import, context-less head refresh, and stale/resolved/reopen lifecycle.
9. Formatting, focused GitLab/model tests, full library tests supported on the
   current host, clippy with warnings denied, `tpatch test`, and
   `tpatch verify` pass.
10. No live provider mutation occurs. The Wayfinder ticket remains open or
    blocked pending the disposable GitLab head-shift rerun.

## Implementation plan

1. Add the validated optional range/native-anchor shape to
   `forge::remote_comments`.
2. Deserialize and validate GitLab line-range endpoints, serialize the
   typed shape while retaining the original raw position value, and classify
   against the current MR head.
3. Pass the current MR head from `list_review_threads`.
4. Extend durable import and thread lifecycle APIs so ranges and stale state
   survive persistence.
5. Update all non-GitLab `RemoteReviewThread` constructors with absent optional
   fields.
6. Add focused provider/model/store tests, then run the repository gates.
7. Record/verify the patch with `tpatch`, update the ticket progress, land one
   commit, and leave live validation explicitly pending.

## Non-goals

- Guessing a new GitLab line/range from nearby content.
- Calling GitLab or recreating the disposable sandbox.
- Wiring durable publication into the TUI (separate ticket).
- Fixing local non-provider anchor relocation (separate ticket).
- Adding native-anchor payloads to other providers in this feature.

## Accepted tradeoff

Per the GitLab capability contract (`StaleAnchorSignal::VersionMismatch`) and
the motivating issue, any non-empty position/current head mismatch is stale
even if a rebase happened not to move that file. The adapter does not infer
semantic equivalence from nearby content.
