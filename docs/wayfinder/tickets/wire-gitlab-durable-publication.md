---
id: wire-gitlab-durable-publication
title: Wire GitLab durable publication into the TUI
type: prototype
mode: AFK
status: blocked
owner:
blocked_by: [classify-gitlab-range-anchors]
---

## Question

Connect the existing GitLab durable create/reply/resolve/reopen operations to
the TUI publication path and persist provider mappings without duplicates.

## Completion

The confirmation plan and execution publish supported durable roots, replies,
and status transitions; successful operations persist provider mappings;
partial failures resume safely; retries do not duplicate work; and mock plus
disposable GitLab tests pass.

## Evidence

The live GitLab 19.2.1 backend lifecycle passed, but the TUI kept durable roots,
replies, and resolution local-only with no provider mapping.
[Private continuation issue #3](https://github.com/jdbencardinop/diffreviewtui/issues/3)
contains the sanitized acceptance criteria.
