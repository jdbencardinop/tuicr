---
id: release-cross-platform-fork
title: Release the cross-platform fork
type: task
mode: AFK
status: open
owner:
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

## Remaining work

- arm64 builds, signing/notarization, and package-manager distribution — not
  started.

No Git tag has been created and nothing has been pushed or published.

## Next work

All tracked validation dependencies are closed. Scope and complete
arm64/signing/distribution work, then replace the current offline candidate
with a real tagged/pushed release. `docs/offline-candidate/README.md` and
`docs/offline-candidate/SECRET-SCAN.md` document the exact accepted scope,
checksums, and secret-scan contract of the current candidate to carry forward
into that release.
