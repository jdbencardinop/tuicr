---
id: release-cross-platform-fork
title: Release the cross-platform fork
type: task
mode: AFK
status: blocked
owner: copilot
blocked_by: [validate-wsl-baseline, validate-live-provider-parity]
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

- real Ubuntu-under-WSL run (`validate-wsl-baseline`);
- live GitHub/GitLab/Azure DevOps mutation validation
  (`validate-live-provider-parity`);
- arm64 builds, signing/notarization, and package-manager distribution — not
  started.

No Git tag has been created and nothing has been pushed or published.

## Unblock condition

Both listed tickets close, then arm64/signing/distribution work is scoped and
done, then a real tagged/pushed release replaces the current offline
candidate. See `retrospectives/offline-release-acceptance.md` in the research
workspace for the exact accepted-vs-not-accepted scope of the current
candidate, and the research workspace's
`docs/follow-on-map/tickets/20-release-cross-platform-fork.md` for the full
history.
