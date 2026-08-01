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
- Integrated offline source: commit `d8b1f63`.
- Offline candidate / implementation-package base: commit `ca319dc` — the
  five-provider offline-validated candidate with fork identity, disabled
  self-update, and an isolated data dir.
- This development branch's tip adds docs-only bootstrap metadata on top of
  `ca319dc` (this handoff doc, `docs/fork/DECISIONS.md`/`PATCHES.md`,
  `docs/wayfinder/`, `docs/fork/AGENT-WORKFLOW.md`, and the tracked
  `.tpatch/` workspace) — no `src/` changes beyond `ca319dc`.
- Reported version: `tuicr 0.19.1-offline-candidate.1+ca319dc`.
- Platforms packaged: macOS x86_64 and Linux x86_64 only.
- Artifact readiness: `OFFLINE_VALIDATED_ONLY` — not a release. See
  `docs/offline-candidate/README.md` and `docs/offline-candidate/SECRET-SCAN.md`
  for the accepted scope and artifact/checksum contract.

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
6. Update the teaching package only after a real release interface is fixed —
   depends on step 4; not tracked as its own ticket here since it strictly
   follows that release.

## Evidence pointers in this repo

- `docs/fork/DECISIONS.md` — accepted decisions with inline rationale.
- `docs/fork/PATCHES.md` — per-feature commit-range and validation index.
- `docs/wayfinder/` — the open/blocking tickets that gate a real release.
- `docs/offline-candidate/` — the packaged candidate itself (install,
  migration, provider capabilities, secret-scan contract).
- `docs/fork/TPATCH.md` — the local tpatch committed-range verifier fix and
  when to apply it.

## Remaining evidence gaps

- actual Ubuntu/WSL run;
- approved GitHub/GitLab/Azure DevOps mutation sandboxes;
- multi-review daily-use observation.

No external issues or review comments have been submitted.
