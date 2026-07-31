# tuicr offline-candidate — @@ARCHIVE_NAME@@

**Status: OFFLINE-VALIDATED ONLY. This is NOT a release.** No Git tag was
created, nothing was pushed, and no GitHub Release was published. This
archive exists so the fork's maintainers can hands-on validate a packaged
binary before any real release decision.

## What this is

A local, reproducible packaging of a small upstream-friendly fork of
[Tuicr](https://github.com/agavra/tuicr) that adds durable provider-neutral
review threads plus Azure DevOps/Gitea/Forgejo adapters. See
`docs/follow-on-map/map.md` and `docs/decisions/final-recommendation.md` in
the research workspace for why this fork exists.

| Field | Value |
| --- | --- |
| Upstream baseline version | `@@VERSION@@` |
| Fork identity (`--version` output) | `tuicr @@FORK_VERSION@@+@@SOURCE_SHA_SHORT@@` |
| Source commit (short) | `@@SOURCE_SHA_SHORT@@` |
| Source commit (full) | `@@SOURCE_SHA_FULL@@` |
| OS / architecture | `@@OS@@` / `@@ARCH@@` |
| Packaged at (UTC) | `@@PACKAGED_AT@@` |
| Packaging tool | `scripts/package-offline-candidate.sh` |

## Binary identity — `--version` now self-identifies as a fork build

Running the packaged binary's `--version` prints
`tuicr @@FORK_VERSION@@+@@SOURCE_SHA_SHORT@@` — a semver prerelease tag
(`-offline-candidate.N`, set in `Cargo.toml`) plus the exact source commit
SHA embedded at build time (`build.rs`). This can never be confused with an
unmodified upstream `tuicr @@VERSION@@` release, which never publishes an
`-offline-candidate` prerelease. Cross-check against `manifest.json`'s
`source.commit_full` for the authoritative source identity.

`Cargo.toml`'s `name`/`repository`/`homepage` fields still point at the
upstream project (`agavra/tuicr`) for provenance — this fork does not claim
to be a different upstream project, only a distinguishable build of it.

## `tuicr update` is disabled in this build (no self-replace risk)

Earlier offline-candidate builds of this fork did not change
`Cargo.toml`'s crate/repository identity, so the binary's automatic
background update check and `tuicr update` subcommand both still targeted
real upstream `agavra/tuicr` on crates.io/GitHub — running them could
silently self-replace the fork binary with unmodified upstream, discarding
the durable-thread feature with no warning. **This build fixes that at the
source level, not just in documentation:**

- The automatic background startup check (`crates.io/api/v1/crates/tuicr`)
  is unconditionally disabled — it never makes a network call, regardless
  of `--no-update-check`/`no_update_check` config (those flags still parse
  for backward compatibility but no longer control anything).
- `tuicr update` (with or without a pinned version) now fails immediately
  with an explicit `tuicr-offline-candidate: ... is disabled` message and
  makes **zero** network calls — it never contacts crates.io, GitHub
  Releases, Homebrew, cargo, or mise. Reinstall manually from a new
  packaged archive instead.
- The real update-installer logic (`src/update/`) is intentionally left
  intact and still covered by its own test suite for potential future
  re-enablement upstream; it is simply never reached from this binary.

`config.no-update-check.toml` (bundled in this archive) is now redundant
for suppressing the startup check specifically (the check is disabled
unconditionally either way) but is still included for anyone who wants the
config-file behavior documented explicitly.

## Fork-specific data/config directories (no upstream collision)

This build resolves its data and config directories under the
fork-specific app id `tuicr-offline-candidate`, **not** upstream's
`tuicr` — see the table in `MIGRATION.md`. A real upstream `tuicr` install
on the same machine no longer shares (or races on) the same reviews/config
directory as this fork build. Use `scripts/import-upstream-reviews.sh` to
explicitly, safely copy existing upstream review data into this fork's
directory (one-way copy, backup-first, never auto-deletes/moves the
upstream source) — see `MIGRATION.md`.


## What's validated vs. not

Validated in this archive:

- `cargo fmt --check`, `cargo clippy -D warnings`, and `cargo test --lib`
  against the exact source commit above.
- A native release build for this OS/architecture, `--locked` against the
  committed `Cargo.lock`.
- The archive extracts cleanly and the binary runs (`--version`, `--help`).
- A local Git fixture smoke: opening a working-tree diff and the
  non-interactive `tuicr review` / `tuicr review thread` / `tuicr review
  publish --dry-run` CLI paths, entirely offline (no network, no external
  provider credentials).

**Not validated, and out of scope for this archive:**

- Live provider mutations against real GitHub, GitLab, Azure DevOps, Gitea,
  or Forgejo instances. Azure DevOps and Gitea/Forgejo support is validated
  only against official API docs and recorded fixtures (see
  `docs/findings/forkability/` and `PROVIDER-CAPABILITIES.md` in this
  archive).
- Ubuntu under real WSL. The Linux binary here was built and smoke-tested
  inside a pinned `linux/amd64` Docker container on macOS — **that is a
  Linux-container pass, not a WSL pass.** WSL has its own filesystem,
  clipboard, and terminal integration quirks that a container cannot
  reproduce.
- Apple Silicon / arm64 or a "universal" macOS binary. This archive is
  x86_64-only on both platforms; no arm64 build is included or implied.
- crates.io publication, a GitHub Release, or any tag/push.

## Explicit blockers (read before assuming anything beyond this scope)

- **No WSL validation.** The Linux artifact is built and smoke-tested in a
  pinned `linux/amd64` Docker container on macOS. That is a Linux-container
  pass, not a real Ubuntu-under-WSL pass — WSL's filesystem, clipboard, and
  terminal integration are not exercised.
- **No live provider validation.** GitHub/GitLab/Azure DevOps/Gitea/Forgejo
  mutation paths are not exercised against real accounts/services from this
  archive; see "Provider capability limitations" below.
- **Gitea/Forgejo scope is offline/fixture-verified only** against the
  specific evidenced versions (1.24.7 / 16.0.1), not a live-service pass.
- **Not signed or notarized.** This binary is not code-signed (macOS) and
  has no Gatekeeper notarization; running it will trigger the standard
  unsigned-binary warning (see `INSTALL.md`).
- **No package-manager install path.** This fork is not published to
  Homebrew, crates.io, an APT/RPM repo, `mise`, or `nix`; manual archive
  extraction (`INSTALL.md`) is the only supported install method.
- **No macOS arm64/Apple Silicon or "universal" binary** — x86_64-only on
  both platforms.
- **This is not a Git tag, GitHub Release, or crates.io publication** —
  offline-validated only.

## `update-test` CI job is not fork-valid (informational only)

This repository's `.github/workflows/ci.yml` has an `update-test` job that
exercises `tuicr update` end-to-end against **real upstream `agavra/tuicr`
GitHub Releases** (downloading real release assets and replacing a real
installed binary). That job is **not valid for this offline fork
candidate**: this build's `tuicr update` is intentionally disabled (see
above) and its `--version` output no longer matches an upstream release
tag the job expects. This is documented here only — the CI workflow itself
was intentionally left unmodified, since fixing/repurposing upstream CI is
out of scope for a local offline-candidate packaging pass.

## Provider capability limitations

See `PROVIDER-CAPABILITIES.md` in this archive for the full matrix. In short:
GitHub and GitLab have live-verified read paths from prior evaluation work;
Azure DevOps and Gitea/Forgejo adapters are offline/fixture-verified only.
Gitea/Forgejo never support standalone thread creation, reply, or resolve on
the evidenced versions (1.24.7 / 16.0.1) — the tool reports these as
`Unsupported`, never silently drops them.

## Contents of this archive

- `tuicr` — the release binary for `@@OS@@`/`@@ARCH@@`.
- `LICENSE` — upstream MIT license (unmodified).
- `README.md` — this file.
- `INSTALL.md` — install/run instructions.
- `MIGRATION.md` — data location, backup, schema version, and rollback.
- `PROVIDER-CAPABILITIES.md` — provider capability matrix and limitations.
- `config.no-update-check.toml` — sample config documenting
  `no_update_check`; the automatic startup check is disabled unconditionally
  in this build regardless of this file (see above).

## Reproducing this build

```bash
git clone <fork-remote> tuicr-fork
cd tuicr-fork
git checkout @@SOURCE_SHA_FULL@@
scripts/package-offline-candidate.sh --output-dir /path/to/out
```

The script refuses to run against a dirty tracked-source tree, builds from
a clean `git archive` checkout of the exact commit (not the live working
tree, so untracked files can never leak into the build), and records the
exact toolchain, target, and commands used in `manifest.json`.
