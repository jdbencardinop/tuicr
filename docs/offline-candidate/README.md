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
| Upstream/crate version | `@@VERSION@@` |
| Source commit (short) | `@@SOURCE_SHA_SHORT@@` |
| Source commit (full) | `@@SOURCE_SHA_FULL@@` |
| OS / architecture | `@@OS@@` / `@@ARCH@@` |
| Packaged at (UTC) | `@@PACKAGED_AT@@` |
| Packaging tool | `scripts/package-offline-candidate.sh` |

## Binary identity — read this before trusting `--version`

Running the packaged binary's `--version` prints `tuicr @@VERSION@@` —
**identical to unmodified upstream 0.19.1.** The fork does not change
`Cargo.toml`'s version field, so **the binary cannot self-report that it is a
fork build.** Fork identity is established entirely by this archive's
manifest (`manifest.json` alongside the checksums) recording the exact
source commit SHA above, not by anything the binary prints. Do not infer
upstream-vs-fork identity from `--version` output alone; check the manifest
or `tuicr review thread --help` (the `thread`/`publish` subcommands only
exist in this fork).

## Do not run `tuicr update` — self-replace risk with real upstream

This fork does not repoint `Cargo.toml`'s `repository`/crate/binary name
away from upstream. That means the packaged binary's self-update path
(`tuicr update`, and its automatic background version check on every
startup unless `--no-update-check`/`no_update_check = true` is set) still
targets **real upstream `agavra/tuicr`** GitHub Releases and crates.io —
not this fork. Concretely:

- **Every default startup phones home** to `crates.io/api/v1/crates/tuicr`
  in a background thread to check for a newer version, unless you pass
  `--no-update-check` or set `no_update_check = true` in `config.toml`.
- **Running `tuicr update` (or `brew upgrade agavra/tap/tuicr` / `cargo
  install tuicr --force` / `mise upgrade github:agavra/tuicr --bump`) will
  silently fetch and self-replace this fork binary with the real,
  unmodified upstream release** — discarding the fork's durable-thread
  feature entirely, with no warning that anything fork-specific was lost.

**Do not run `tuicr update` or any package-manager upgrade command against
this binary.** A `docs/offline-candidate/config.no-update-check.toml`
sample is included in this archive; copy it to your config directory (see
`MIGRATION.md`) as `config.toml` to at least suppress the automatic
startup check. This does not disable the `update` subcommand itself — that
requires simply not invoking it. This is tracked as its own
release-readiness blocker in `manifest.json`, independent of the
platform/WSL/provider gaps below.

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
- `config.no-update-check.toml` — sample config disabling the automatic
  startup crates.io check (see "Do not run `tuicr update`" above).

## Reproducing this build

```bash
git clone <fork-remote> tuicr-fork
cd tuicr-fork
git checkout @@SOURCE_SHA_FULL@@
scripts/package-offline-candidate.sh --output-dir /path/to/out
```

The script refuses to run against a dirty tracked-source tree and records
the exact toolchain, target, and commands used in `manifest.json`.
