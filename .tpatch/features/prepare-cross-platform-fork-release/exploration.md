# Exploration: prepare-cross-platform-fork-release

## Existing release surfaces

- `.github/workflows/release.yml` tags and publishes crates.io in `publish`
  before the binary matrix. It is inherited upstream behavior and unsafe for
  this fork.
- The binary version, build SHA variable, update-disabled text, and app
  directories all contain `offline-candidate`.
- `scripts/package-offline-candidate.sh` is a large historical evidence
  packager for x86_64-only archives. It should remain an archived reproducer,
  not become the hosted release path.
- `flake.nix` already exposes all systems supported by `flake-utils`; Cargo
  supports installation directly from this fork's Git tag without crates.io.
- No GitHub workflows are currently registered in the fork because the
  default branch has never executed Actions. A pull request from the
  development branch can exercise the new candidate workflow.

## Minimal changeset

- Identity: `Cargo.toml`, `Cargo.lock`, `build.rs`, `src/cli.rs`,
  `src/main.rs`, `src/config/mod.rs`, `src/persistence/storage.rs`, and
  focused tests.
- Migration: `scripts/import-upstream-reviews.sh`.
- Packaging: new `scripts/package-release-artifact.sh` and
  `.github/workflows/release-candidate.yml`.
- Release contract: `README.md`, `RELEASE.md`, `docs/release/`, fork
  decisions/patch index, Wayfinder ticket/map, and handoff.

The provider, review-store schema, forge behavior, and TUI feature surface do
not need changes.
