---
id: provision-provider-sandboxes
title: Provision approved provider sandboxes
type: task
mode: HITL
status: blocked
owner: copilot
blocked_by: []
---

## Question

Create or approve disposable GitHub, GitLab, and Azure DevOps test targets so
live mutation parity can be proven, alongside the local version-pinned
Gitea/Forgejo instances already in place.

## Completion

Each target has minimum-scope credentials, teardown instructions, no private
source, and explicit permission for automated comment/review writes.

## Status

- **Done:** local, version-pinned **Gitea 1.24** and **Forgejo 16** —
  verified 2026-07-30 against reproducible, disposable Docker fixtures with
  automatic credential/token generation and teardown.
- **Open/HITL-blocked:** disposable **GitHub**, **GitLab**, and **Azure
  DevOps** sandboxes. No approval has been given for any of the three; no
  external target has been created or contacted.

This single gap is what keeps the following blocked, downstream:

- `docs/wayfinder/tickets/validate-live-provider-parity.md` (GitHub/GitLab
  mutation parity and the Azure DevOps adapter's live validation);
- `docs/wayfinder/tickets/release-cross-platform-fork.md` (a release needs
  the above closed first).

## Unblock condition

Explicit approval plus minimum-scope, teardown-documented disposable GitHub,
GitLab, and Azure DevOps targets. Record the exact scopes, teardown steps,
and approval directly in this ticket's Resolution section when granted.
