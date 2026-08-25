---
id: fix-github-head-refresh
title: Preserve GitHub diff and comments across head refresh
type: implementation
mode: autonomous
status: open
owner:
blocked_by: []
---

## Question

Why does a GitHub pull-request reload/restart render an empty diff and omit a
provider-relocated inline comment after the pull-request head advances, even
though GitHub returns the updated diff and current comment position?

## Completion

- In-process reload and fresh restart retain the updated pull-request diff.
- A provider comment relocated from line 42 to line 47 is imported and
  rendered once with its original/current commit metadata preserved.
- Durable local threads reconcile against the new head without duplication.
- Tests reproduce the exact base/review/anchor-shift fixture history.
- The real disposable GitHub lifecycle rerun passes after the fix.

## Evidence

The 2026-08-25 live run observed GitHub returning a valid 14-file, 1,005-line
diff and relocating the comment to line 47. Tuicr reload and restart both
rendered an empty file pane and omitted that comment. Sanitized evidence:
`../../../../artifacts/validation/2026-08-25-wsl-github/`.
