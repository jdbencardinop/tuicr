# Specification: prepare-cross-platform-fork-release

## Acceptance Criteria

1. Promote package identity from `offline-candidate` to `fork` prerelease and
   embed the source SHA through a release-neutral build variable.
2. Move default data/config paths to `tuicr-fork`; cover Linux, macOS, and
   Windows path routing with tests.
3. Generalize the import script so `upstream` and `offline-candidate` are
   named, explicit sources and destination backup behavior is unchanged.
4. Add one native artifact packager shared by local and CI builds.
5. Add a candidate workflow whose matrix is:
   `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
   `x86_64-apple-darwin`, and `aarch64-apple-darwin`.
6. Build on matching native runners, run archive smoke checks, emit artifact
   provenance and checksums, and verify the combined set in an aggregate job.
7. Never publish crates.io, create tags, or create GitHub Releases in the
   candidate workflow.
8. Document GitHub archives as the eventual primary channel, with Cargo git
   and Nix as package-manager-capable source installs.
9. State precisely that macOS artifacts are ad-hoc signed, not Developer ID
   signed or notarized, until approved credentials exist.

## Implementation Plan

1. Refactor identity constants and migration script behavior with focused
   tests.
2. Add deterministic native packaging and a four-target candidate workflow.
3. Replace the inherited unsafe release instructions with fork promotion
   gates and distribution/signing policy.
4. Run local Rust/script/document checks.
5. Record and land the feature through `tpatch`.
6. Push the development branch and use a draft PR to execute native arm64
   candidate jobs; do not tag or publish.
