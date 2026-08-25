---
id: classify-local-anchor-shifts
title: Classify local anchors after head shifts
type: prototype
mode: AFK
status: closed
owner: copilot
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

## Resolution

**Closed 2026-08-25 — pass.** TUI-created local line/range threads now capture
bounded provider-neutral context from the exact displayed old/new side.
Startup, local `:e`, source-selection changes, and PR head advances re-evaluate
context-bearing anchors against trustworthy side content. One exact match
relocates the complete anchor; zero or multiple matches retain the last known
legacy row with the existing stale/ambiguous status, never a nearest-line
guess.

Durable threads remain canonical, while uniquely relocated current anchors
re-key their legacy compatibility shadows so rendering, navigation, CLI
inspection, and persistence agree. Migration recognizes the immutable root
comment ID before the line-derived legacy thread ID, preventing a relocated
shadow from creating a duplicate thread on the next load.

Focused regressions cover file, line, range, old/new side, 42-to-47 movement,
zero/multiple matches, cold startup, local reload, visible annotations,
persistence, and repeat migration. Formatting, clippy with warnings denied,
and the 1,666-test locked library suite supported by Ubuntu Git 2.43 passed;
the only excluded tests are the two already-documented Git 2.48 fixtures.

The real Ubuntu 24.04/WSL2 TUI rerun used the public synthetic fixture and
isolated XDG state. After a TUI-authored comment on
`policy_rule_042` at line 42, the five-line shift plus `:e` showed
`policy_rule_037` unannotated at line 42 and the comment attached to
`policy_rule_042` at line 47. After exit, `review comments` and
`review thread list` each reported line 47 with one open thread. Raw local
output remains only under the ignored
`artifacts/raw/local-anchor-20260825/` superproject path.
