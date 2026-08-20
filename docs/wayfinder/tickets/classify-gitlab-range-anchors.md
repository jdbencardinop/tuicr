---
id: classify-gitlab-range-anchors
title: Classify GitLab range anchors after head shifts
type: prototype
mode: AFK
status: closed
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

## Resolution

The 2026-08-20 disposable GitLab 19.2.1 rerun exposed a second native stale
shape not represented by the offline fixtures. After the five-line shift,
GitLab preserved `line_range` 70-72, relocated the root `new_line` to 77, and
rewrote the position `head_sha` to the current MR head. Commit `d5cbbdd`
therefore extends the `classify-gitlab-range-anchors` feature to preserve a
valid same-side ordered range while treating a provider-terminal mismatch as
stale evidence; current consistent ranges remain strict, and malformed
cross-side/reversed ranges remain untrusted.

The committed live harness passed create/reply/resolve/reopen and then verified
`range_start=70`, `range_end=72`, `is_outdated=true`,
`native_head_differs=false`, and `native_terminal_differs=true`. The default
TUI unresolved view hid the thread; `:comments all` showed the marked thread
muted as `outdated` and locally `stale`; `:e` preserved that result. The
durable `ReviewStore` retained a stale range anchor at 70-72 with
`provider_mappings.gitlab.is_outdated=true`.

Formatting, clippy, 62 focused GitLab tests, live backend verification, TUI
reload, durable-store inspection, and exact sandbox teardown passed. The
tracked `tpatch` feature was landed and verified at `d5cbbdd`.
