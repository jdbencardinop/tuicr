---
id: upstream-and-reconcile
title: Upstream changes and reconcile the fork
type: task
mode: HITL
status: blocked
owner: copilot
blocked_by: [release-cross-platform-fork]
---

## Question

Split the fork's general-purpose changes into small, reviewable upstream PRs,
and keep only the customizations upstream will not accept active in `tpatch`.

## Completion

Every `tpatch` feature links to an upstream discussion/PR, adopted patches
retire cleanly, remaining patches verify against current upstream, and the
fork rebases without manual undocumented changes.

## Status

**Nothing has been posted upstream.** One feature is ready and waiting on
approval to post:

- `expose-review-comment-authors` — local upstream-ready source commit
  `6ea2048`, targeted formatting/test evidence and a PR draft are complete,
  but posting requires explicit approval and an upstream-contribution
  decision. See `docs/fork/PATCHES.md` for the commit and
  `docs/wayfinder/tickets/README.md` for why the fuller narrative stays in
  the research workspace's
  `docs/follow-on-map/tickets/01-upstream-author-json.md`.

Every other `tpatch` feature (`editable-provider-neutral-threads`,
`provider-capabilities`, `gitea-forgejo-adapter`,
`azure-devops-adapter-offline`, `tui-thread-integration`,
`github-gitlab-thread-parity-offline`) remains fork-only with no upstream
split decided yet — see `docs/fork/PATCHES.md`'s "Upstream target" column for
the current disposition of each.

## Unblock condition

This ticket depends on `release-cross-platform-fork` closing first (a stable
release interface makes it clear what actually needs to remain a fork patch
vs. what can be proposed upstream as-is). Independently, the one ready
patch (`expose-review-comment-authors`) only needs an explicit go-ahead to
post — it does not need to wait for the release.
