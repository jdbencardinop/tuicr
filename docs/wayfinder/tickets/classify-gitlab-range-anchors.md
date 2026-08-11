---
id: classify-gitlab-range-anchors
title: Classify GitLab range anchors after head shifts
type: prototype
mode: AFK
status: open
owner:
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
