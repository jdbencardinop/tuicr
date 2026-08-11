---
id: classify-local-anchor-shifts
title: Classify local anchors after head shifts
type: prototype
mode: AFK
status: open
owner:
blocked_by: []
---

## Question

Make local durable comments relocate to the same semantic code or become
explicitly stale/ambiguous after a head shift, never silently remain on the
wrong numeric line.

## Completion

The common five-line insertion fixture moves the line-42 comment to the
intended semantic line 47 or reports a typed stale/ambiguous state. File,
single-line, old/new-side, and range anchors have deterministic regression
tests, and the fixed behavior is rerun in the real WSL TUI.

## Evidence

The 2026-08-11 WSL run observed both upstream and the fork rendering the saved
line-42 comment on `policy_rule_037` after `policy_rule_042` moved to line 47.
See `validate-wsl-baseline.md`.
