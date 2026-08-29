# Analysis: prepare-cross-platform-fork-release

## Summary

The maintained fork is live-validated but still identifies itself and stores
data as an offline-only candidate. The inherited release workflow publishes
the `tuicr` crate and creates a tag before any binary matrix completes, so it
must not be used for this fork. The host is x86_64 WSL and Docker has no arm64
emulation; native Linux and macOS arm64 validation therefore belongs on
GitHub-hosted arm64 runners after the branch is pushed.

## Compatibility

**Status:** compatible as a prerelease with an explicit data migration.

Use a fork-specific prerelease version and application ID, retain the same
`tuicr` executable name, and keep self-update disabled until the fork owns a
stable release channel. Existing upstream and offline-candidate stores must
remain untouched and importable through a backup-first one-way copy.

## Affected Areas

- Cargo/build/CLI identity and fork repository metadata.
- Data/config directory selection and migration tests.
- Native release artifact packaging and GitHub Actions.
- Installation, migration, capability, signing, rollback, and distribution
  documentation.

## Acceptance Criteria

1. The binary reports a fork prerelease version plus exact source SHA.
2. Fork data/config uses `tuicr-fork`, never upstream `tuicr` or the archived
   `tuicr-offline-candidate` directory.
3. Upstream and prior-candidate data can be imported by an explicit,
   backup-first, one-way command.
4. A non-publishing workflow builds and smoke-tests native Linux/macOS
   x86_64 and arm64 archives before any tag or hosted release exists.
5. Every archive carries license, install/migration/capability docs,
   provenance, and SHA-256 verification.
6. crates.io publication remains disabled; Nix and Cargo git installs are
   documented against this fork.
7. Apple Developer ID signing/notarization is a visible credential gate. The
   workflow may ad-hoc sign macOS binaries but must not claim notarization.
8. No tag, GitHub Release, package publication, or upstream mutation occurs
   during candidate preparation.

## Unresolved Questions

- The first hosted prerelease tag remains a user-approved promotion step after
  the native matrix passes.
- Developer ID signing/notarization requires credentials not present in this
  repository and cannot be validated from WSL.
