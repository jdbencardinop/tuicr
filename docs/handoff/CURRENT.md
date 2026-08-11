# Current handoff

## Status

Exploration, offline implementation, and the real Ubuntu/WSL baseline are
complete. A disposable self-hosted GitLab run is also complete. Release and
upstream submission remain blocked on approved GitHub/Azure DevOps sandboxes,
GitLab durable-publication gaps, and explicit disposition of the WSL caveats
recorded below.

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
- Validation-record commit: `ecae453` — docs-only WSL/GitLab findings; no
  `src/` changes.
- This development branch's tip adds docs-only bootstrap metadata on top of
  `ca319dc` (this handoff doc, `docs/fork/DECISIONS.md`/`PATCHES.md`,
  `docs/wayfinder/`, `docs/fork/AGENT-WORKFLOW.md`, and the tracked
  `.tpatch/` workspace) — no `src/` changes beyond `ca319dc`.
- Latest build exercised on WSL reports
  `tuicr 0.19.1-offline-candidate.1+c2a7554`; later validation/tracker commits
  only change documentation and have not been rebuilt separately. The
  archived implementation package at `ca319dc` reports the corresponding
  `+ca319dc` identity.
- Platforms packaged: macOS x86_64 and Linux x86_64 only.
- Artifact readiness: `OFFLINE_VALIDATED_ONLY` — not a release. See
  `docs/offline-candidate/README.md` and `docs/offline-candidate/SECRET-SCAN.md`
  for the accepted scope and artifact/checksum contract.
- Real Ubuntu 24.04 under WSL2: **pass with caveats** for upstream `v0.19.1`
  and fork tip `c2a7554`; see
  `docs/wayfinder/tickets/validate-wsl-baseline.md`.
- Self-hosted GitLab 19.2.1: legacy comment/approve/request-changes and direct
  durable adapter operations live-pass; TUI durable publication and range
  stale classification remain gaps. See
  `docs/wayfinder/tickets/validate-live-provider-parity.md`.

Per-feature commit ranges and validation state:
`docs/fork/PATCHES.md`.

## Blocked frontier

In dependency order:

1. Fix and retest unsafe anchor handling for local comments and GitLab durable
   ranges — `docs/wayfinder/tickets/classify-local-anchor-shifts.md` and
   `docs/wayfinder/tickets/classify-gitlab-range-anchors.md`.
2. Wire and retest GitLab durable TUI publication —
   `docs/wayfinder/tickets/wire-gitlab-durable-publication.md`.
3. Approve/provision disposable GitHub and Azure DevOps sandboxes, then
   execute their remaining live parity —
   `docs/wayfinder/tickets/provision-provider-sandboxes.md` and
   `docs/wayfinder/tickets/validate-live-provider-parity.md`.
4. Explicitly accept or fix the WSL TUI-clipboard boundary —
   `docs/wayfinder/tickets/decide-wsl-clipboard-boundary.md`.
5. Add arm64, signing/notarization, package-manager distribution, and cut an
   actual tagged/pushed release — depends on 1–4;
   `docs/wayfinder/tickets/release-cross-platform-fork.md`.
6. Post the one ready upstream patch (comment-author JSON) once approved, and
   reconcile the rest after the release interface stabilizes —
   `docs/wayfinder/tickets/upstream-and-reconcile.md`.
7. Update the teaching package only after a real release interface is fixed —
   depends on step 5; not tracked as its own ticket here since it strictly
   follows that release.

## Evidence pointers in this repo

- `docs/fork/DECISIONS.md` — accepted decisions with inline rationale.
- `docs/fork/PATCHES.md` — per-feature commit-range and validation index.
- `docs/wayfinder/` — the open/blocking tickets that gate a real release.
- `docs/offline-candidate/` — the packaged candidate itself (install,
  migration, provider capabilities, secret-scan contract).
- `docs/fork/TPATCH.md` — the local tpatch committed-range verifier fix and
  when to apply it.

## Known WSL caveats

- line anchors retain their numeric position after the five-line fixture
  shift instead of relocating or becoming stale;
- TUI clipboard export does not reach the Windows clipboard; `--stdout` and
  direct `clip.exe` work;
- Ubuntu Git 2.43 cannot execute two newer-Git fixture tests;
- `wslview` misdetects interop although PowerShell browser launch works.

GitLab remains out of scope for the WSL baseline itself. Its separate
self-hosted provider run is complete with the gaps noted above.

## Remaining evidence gaps

- approved GitHub/Azure DevOps mutation sandboxes;
- GitLab TUI durable-publication and range-staleness behavior;
- durable sanitized WSL/GitLab run artifacts in shared Git history;
- multi-review daily-use observation.

The approved disposable GitLab target was mutated and fully torn down. Three
private continuation issues were opened and are mirrored by the fork tickets
above. No GitHub/Azure review mutation or upstream Tuicr issue/PR was
submitted.
