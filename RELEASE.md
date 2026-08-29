# Maintained fork release process

The fork uses a two-stage release. Candidate validation cannot tag, publish a
crate, or create a GitHub Release.

## 1. Validate a candidate

Push the exact source commit and run **Release candidate**. It builds and
smoke-tests native archives on:

- Ubuntu x86_64 and arm64;
- macOS x86_64 and Apple Silicon.

Each archive contains the binary, license, install/migration/provider
documentation, import helper, and provenance. The packaging script performs a
byte-aware secret-pattern scan and emits an adjacent SHA-256 file. The
aggregate job requires all four targets and verifies every checksum.

macOS binaries are ad-hoc signed for integrity testing. They are **not**
Developer ID signed or notarized. A Developer ID Application certificate,
App Store Connect issuer/key, and an approved credential-handling design are
required before claiming Gatekeeper-ready distribution.

## 2. Promote the verified commit

Promotion is a separate, manually dispatched **Promote verified candidate**
workflow protected by the `release` environment. It accepts the successful
candidate run ID and version, then verifies:

- candidate conclusion is `success`;
- candidate workflow path is exactly the release-candidate workflow;
- candidate source SHA equals the commit being promoted;
- Cargo version equals the requested version;
- all four archives and checksums are present and valid.

Only then does it create an annotated tag and a **draft prerelease**. Publishing
that draft is a separate explicit GitHub action. Do not run promotion without
authorization to create the tag and hosted draft.

## Distribution policy

- GitHub checksummed archives are the primary binary channel.
- Cargo installs use an exact Git tag from this fork; this fork does not
  publish the upstream-owned `tuicr` crate to crates.io.
- Nix installs use the exact fork tag through the repository flake.
- Homebrew, Mise, APT, and RPM are not first-release channels.
- `tuicr update` remains disabled until the fork owns a stable, verified
  update manifest and rollback channel.
