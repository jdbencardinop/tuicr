---
id: preserve-azure-native-anchors
title: Preserve Azure DevOps iteration-native anchors
type: implementation
mode: autonomous
status: in-progress
owner: copilot
blocked_by: []
---

## Question

Preserve Azure DevOps `threadContext` and `pullRequestThreadContext` through
`RemoteReviewThread.provider_native_anchor` and durable mappings so a head
update can retain iteration/tracking evidence instead of reducing the thread
to an outdated line.

## Completion

- Azure thread and pull-request iteration context serialize into the opaque
  provider-native anchor.
- Durable import persists the native payload without private source content.
- Head refresh retains iteration IDs, tracking criteria, path/side/range, and
  stale state without guessing relocation.
- Repeat import and restart do not duplicate the thread.
- Mock tests reproduce the live 200-line plus five-line-shift fixture.
- The disposable live TUI/ReviewStore lifecycle passes after the fix.

## Evidence

The 2026-08-25 live adapter run correctly classified the shifted thread as
outdated but returned no provider-native anchor despite Azure supplying thread
and iteration context. Sanitized evidence:
`../../../../artifacts/validation/2026-08-25-wsl-azure-devops/`.
