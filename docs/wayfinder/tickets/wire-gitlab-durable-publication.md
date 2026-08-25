---
id: wire-gitlab-durable-publication
title: Wire GitLab durable publication into the TUI
type: prototype
mode: AFK
status: closed
owner: copilot
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

The real TUI published one durable root, one reply, resolve, and reopen against
the approved disposable GitLab 19.2.1 target. Restarted ReviewStore state
contained one provider-mapped reconciled thread, and a repeated
`:submit comment` was a no-op. Mock coverage forces a failure after a
successful prefix and verifies retry sends only remaining operations.

Sanitized evidence is tracked by the Diffreviewtui superproject at
`artifacts/validation/2026-08-25-wsl-gitlab-publication/`.

## Resolution

Closed. GitLab `:submit comment` now previews and executes durable roots,
replies, resolve, and reopen through the existing planner/executor. Each
successful mutation checkpoints its provider mapping and matching legacy
lifecycle before the next mutation; remote re-import reconciles by provider ID;
worker-side recovery preserves mappings across reload races; and retry planning
does not duplicate completed work.

GitLab review bodies, drafts, approvals, request-changes, resolver work, and
hidden grouped-root cases intentionally retain the legacy path because
GitLab's review-level call is internally multi-step and cannot honestly meet
the per-operation checkpoint contract yet.
