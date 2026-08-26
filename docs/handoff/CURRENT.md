# Current handoff

## Status

Exploration, offline implementation, the real Ubuntu/WSL baseline, local
anchor relocation, and disposable GitLab/GitHub/Azure DevOps runs are
complete. Azure native-anchor preservation now passes the live adapter,
durable-store, real-TUI, and non-draft vote paths. Release and upstream
submission remain blocked on GitHub live-parity fixes and explicit disposition
of the remaining WSL caveats recorded below.

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
- Current source customization: `d5cbbdd` — live-verified GitLab
  position-version/range/native-anchor classification under the tracked
  `classify-gitlab-range-anchors` feature. It preserves valid stale ranges
  when GitLab rewrites the native head but relocates the terminal line.
- Local-anchor customization: `59faa80` — TUI-created local line/range
  anchors capture exact-side context, relocate only on a unique match, persist
  the canonical row across reload/reopen, and become stale/ambiguous without
  nearest-line guessing.
- GitLab durable TUI publication: `9330ee0`, `e6d6d80` — `:submit comment`
  previews and checkpoints durable roots, replies, resolve, and reopen;
  provider-ID re-import reconciliation and retry planning prevent duplicates.
- Azure native-anchor preservation: `c3d6dde` — selected-side ranges and
  provider-native thread/iteration/tracking context survive live adapter
  conversion, stale classification, ReviewStore reload, and duplicate-free
  repeat import.
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
- Self-hosted GitLab 19.2.1: legacy comment/approve/request-changes,
  stale-range classification, and durable TUI root/reply/resolve/reopen
  publication live-pass.
  See
  `docs/wayfinder/tickets/validate-live-provider-parity.md`.

Per-feature commit ranges and validation state:
`docs/fork/PATCHES.md`.

## Blocked frontier

In dependency order:

1. Fix the GitHub durable-publication/head-refresh failures, then execute the
   remaining live parity —
   `docs/wayfinder/tickets/validate-live-provider-parity.md`.
2. Explicitly accept or fix the WSL TUI-clipboard boundary —
   `docs/wayfinder/tickets/decide-wsl-clipboard-boundary.md`.
3. Add arm64, signing/notarization, package-manager distribution, and cut an
   actual tagged/pushed release — depends on 1–3;
   `docs/wayfinder/tickets/release-cross-platform-fork.md`.
4. Post the one ready upstream patch (comment-author JSON) once approved, and
   reconcile the rest after the release interface stabilizes —
   `docs/wayfinder/tickets/upstream-and-reconcile.md`.
5. Update the teaching package only after a real release interface is fixed —
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

## Known WSL caveats

- TUI clipboard export does not reach the Windows clipboard; `--stdout` and
  direct `clip.exe` work;
- Ubuntu Git 2.43 cannot execute two newer-Git fixture tests;
- `wslview` misdetects interop although PowerShell browser launch works.

GitLab remains out of scope for the WSL baseline itself. Its separate
self-hosted provider run is complete with the gaps noted above.

## Remaining evidence gaps

- GitHub durable TUI publication and post-head-update rendering;
- independent GitHub review-state validation with a second standard identity;
- multi-review daily-use observation.

The approved disposable GitLab target was mutated and fully torn down. The
disposable GitHub target was mutated, closed, and deleted. The disposable
Azure DevOps draft PR was mutated, abandoned, and its branch deleted. Three
private continuation issues were opened and are mirrored by the fork tickets
above. No upstream Tuicr issue/PR was submitted.
