---
id: release-cross-platform-fork
title: Release the cross-platform fork
type: task
mode: AFK
status: in-progress
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

## Remaining work

- Native candidate CI passes for Linux/macOS x86_64 and arm64.
- macOS artifacts are ad-hoc signed. Developer ID signing and Apple
  notarization require credentials and an approved secret-handling boundary;
  the first prerelease must label this limitation rather than imply it passed.
- GitHub archives are the primary binary channel. Exact-tag Cargo git and Nix
  installs are documented; crates.io, Homebrew, Mise, APT, and RPM are not
  first-release channels.

No Git tag has been created and nothing has been pushed or published.

## Next work

The candidate is ready for promotion. Promotion requires separate
authorization to create a tag and draft GitHub prerelease. Publishing that
draft remains a second explicit action.

## Release design

The maintained fork now reports `0.19.1-fork.1+<source-sha>`, stores data under
`tuicr-fork`, and imports upstream or archived-candidate sessions through an
explicit backup-first copy. The inherited workflow that tagged and published
crates.io before binary completion was replaced:

- `Release candidate` has read-only permissions and cannot tag or publish;
- each native runner packages one archive and runs it locally;
- the aggregate job requires all four targets and verifies every checksum;
- `Promote verified candidate` accepts only a successful candidate run for the
  exact source commit/version, then creates an annotated tag and **draft**
  prerelease behind the `release` environment;
- no workflow publishes the upstream-owned crates.io package.

The local WSL x86_64 release archive passed build, identity, help, byte-aware
secret-pattern scan, extraction, content, and checksum checks.

The first native run (`33226861714`) passed Linux x86_64 and both macOS
architectures. Linux arm64 built but stopped before upload on a deterministic
56-byte GitHub-token-shaped binary sequence. An exact rerun reproduced SHA-256
`b693954df023f3cd9a2c073a2821437acd59a1ab6623229872259262a488b726`.
The build receives no provider credentials, the other architectures do not
contain the match, and the allowance is now limited to that complete digest;
see `../../release/SECURITY.md`.

### Native candidate result

GitHub Actions run `33227880182` at `7e1f0a4` passed all four native build,
smoke, scan, package, and upload jobs plus the aggregate checksum/coverage
gate. Downloaded evidence independently confirmed Mach-O x86_64/arm64 and ELF
x86_64/aarch64 binaries, matching provenance, required documents, and all
four SHA-256 files.

The inherited Nix job failed three times on crates.io HTTP 403 responses for
different dependencies, while direct Cargo source installation passed. Nix is
therefore deferred and its workflow is manual/non-release; exact-tag Cargo git
is the first package-manager channel.
