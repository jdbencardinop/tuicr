---
id: release-cross-platform-fork
title: Release the cross-platform fork
type: task
mode: AFK
status: blocked
owner: copilot
blocked_by: [validate-live-provider-parity, classify-local-anchor-shifts, decide-wsl-clipboard-boundary]
---

## Question

Integrate and package the smallest fork release that passes the common
review fixture and the full provider contract — as an actual release, not the
current offline-only candidate.

## Completion

macOS and Ubuntu/WSL binaries, upgrade/rollback, data migration/backup,
provider capability docs, security review, benchmark comparison, and
checksums, published as a real (tagged, pushed) release.

## Status

Offline integration and local packaging are **complete**, but this is
explicitly **not a release**:

- integrated offline source: commit `d8b1f63` (`tpatch` feature
  `offline-integration`);
- offline-only candidate source: commit `ca319dc` (`tpatch` feature
  `offline-release-candidate`), readiness `OFFLINE_VALIDATED_ONLY`;
- macOS/Linux x86_64 archives, checksums, manifest, license summary, and a
  binary-aware secret scan exist for this candidate only.

## Blockers

- unsafe local anchor behavior after a head shift
  (`classify-local-anchor-shifts`);
- live GitHub/Azure DevOps mutation validation plus GitLab durable publication
  and range-staleness fixes (`validate-live-provider-parity`);
- explicit acceptance or repair of the WSL TUI clipboard behavior
  (`decide-wsl-clipboard-boundary`);
- arm64 builds, signing/notarization, and package-manager distribution — not
  started.

No Git tag has been created and nothing has been pushed or published.

## Unblock condition

All listed dependencies close, then arm64/signing/distribution work is scoped
and done, then a real tagged/pushed release replaces the current offline
candidate. `docs/offline-candidate/README.md` and
`docs/offline-candidate/SECRET-SCAN.md` document the exact accepted scope,
checksums, and secret-scan contract of the current candidate to carry forward
into that release.
