# Current handoff

## Status

Exploration and offline implementation are complete. Release and upstream
submission are blocked on external access (real WSL, and approved
GitHub/GitLab/Azure DevOps sandboxes).

## Decision

Use Tuicr immediately for local/GitHub/GitLab review. Maintain a small
upstream-friendly, `tpatch`-tracked Tuicr fork for durable provider-neutral
threads plus Azure DevOps/Gitea/Forgejo adapters. Keep Neovim/codediff as a
context/LSP companion only — never a second canonical review store.

Full decision text: `docs/fork/DECISIONS.md`.

## Exact source state

- Upstream fork branch point: `f92502bfbbd172ceb3c16c3dfd1348b52e142be5`
  (rebase to current upstream before further implementation).
- Integrated offline source: `offline-integration`, commit `d8b1f63`.
- Offline candidate (current tip): `offline-release-candidate`, commit
  `ca319dc`.
- Reported version: `tuicr 0.19.1-offline-candidate.1+ca319dc`.
- Platforms packaged: macOS x86_64 and Linux x86_64 only.
- Artifact readiness: `OFFLINE_VALIDATED_ONLY` — not a release. See
  `retrospectives/offline-release-acceptance.md` in the research workspace
  for the full accepted/non-accepted scope and artifact checksums.

Per-feature commit ranges and validation state:
`docs/fork/PATCHES.md`.

## Blocked frontier

In dependency order:

1. Run the baseline and candidate inside actual Ubuntu under WSL —
   `docs/wayfinder/tickets/validate-wsl-baseline.md`.
2. Approve/provision disposable GitHub, GitLab, and Azure DevOps sandboxes
   (local Gitea 1.24/Forgejo 16 are already done) —
   `docs/wayfinder/tickets/provision-provider-sandboxes.md`.
3. Execute live provider mutation parity and teardown against those sandboxes
   — `docs/wayfinder/tickets/validate-live-provider-parity.md`.
4. Add arm64, signing/notarization, package-manager distribution, and cut an
   actual tagged/pushed release — depends on 1–3;
   `docs/wayfinder/tickets/release-cross-platform-fork.md`.
5. Post the one ready upstream patch (comment-author JSON) once approved, and
   reconcile the rest after the release interface stabilizes —
   `docs/wayfinder/tickets/upstream-and-reconcile.md`.
6. Update the teaching package only after a real release interface is fixed
   (research workspace `docs/follow-on-map/tickets/22-update-teaching-package.md`;
   not tracked as its own ticket here since it strictly follows step 4).

## Research and evidence pointers

Kept in the read-only research workspace, not copied into this fork
worktree:

- `docs/decisions/` — accepted decisions and full rationale.
- `docs/follow-on-map/` — full implementation-frontier ticket history.
- `docs/findings/` — candidate/provider/benchmark evidence.
- `docs/evaluation/` — evaluation contract and checklists.
- `fixtures/` — the common review fixture and provider fixtures.
- `retrospectives/` — tool retrospectives and the offline-release acceptance
  record.

This fork worktree's own `docs/offline-candidate/` documents the packaged
candidate itself (install, migration, provider capabilities, secret-scan
contract) and is not duplicated here.

## Remaining evidence gaps

- actual Ubuntu/WSL run;
- approved GitHub/GitLab/Azure DevOps mutation sandboxes;
- multi-review daily-use observation.

No external issues or review comments have been submitted.
