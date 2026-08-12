---
id: classify-gitlab-range-anchors
title: Classify GitLab range anchors after head shifts
type: prototype
mode: AFK
status: blocked
owner: copilot
blocked_by: []
---

## Question

Preserve a GitLab durable range only while its provider-native position is
current; otherwise relocate it from provider evidence or classify it
stale/outdated without guessing.

## Completion

GitLab discussion head/position metadata is compared with the current MR head,
range start/end remain same-side and ordered, and regression tests cover
current ranges, relocated legacy lines, and stale ranges. The disposable
GitLab head-shift fixture passes after the fix.

## Evidence

The live GitLab 19.2.1 run relocated a legacy line from 42 to 47 but left a
durable range at 70-72 and imported it with `is_outdated = false`.
[Private continuation issue #1](https://github.com/jdbencardinop/diffreviewtui/issues/1)
contains the sanitized acceptance criteria.

## Progress

Offline implementation is claimed under the `tpatch` feature
`classify-gitlab-range-anchors`. It will classify position-head mismatches,
retain validated same-side range endpoints and the opaque native position in
durable mappings, and propagate provider-outdated state into the durable
thread lifecycle. The ticket stays open until the disposable GitLab head-shift
fixture is rerun.

The offline implementation is complete: valid old/new/context ranges are
retained, malformed ranges are outdated without coercion, raw provider
positions survive in mappings, and stale state is monotonic across first
import, repeat import, resolve/reopen, and context-less head refresh. Formatting
and clippy passed; 61 focused GitLab tests and all 1,655 supported locked
library tests passed. Only the documented libgit2 relative-worktree environment
test was filtered.

## Unblock condition

Recreate the approved disposable GitLab fixture, repeat the five-line head
shift, and confirm the durable range is reported stale/outdated (never current
at 70-72) through both backend and TUI reload paths. Record the sanitized result
here before closing the ticket.
