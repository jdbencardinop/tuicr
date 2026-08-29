# Implementation Record: prepare-cross-platform-fork-release

**Recorded**: 2026-08-29T01:32:12Z
**Files changed**: 24
**Patch size**: 61544 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .github/workflows/ci.yml                           |  49 +----
 .github/workflows/release.yml                      | 232 +++++----------------
 .tpatch/FEATURES.md                                |   1 +
 Cargo.lock                                         |   2 +-
 Cargo.toml                                         |  15 +-
 README.md                                          |  49 ++---
 RELEASE.md                                         | 110 ++++------
 build.rs                                           |  23 +-
 docs/fork/AGENT-WORKFLOW.md                        |  18 +-
 docs/fork/DECISIONS.md                             |  27 ++-
 docs/handoff/CURRENT.md                            |  23 +-
 docs/wayfinder/map.md                              |  10 +-
 .../tickets/release-cross-platform-fork.md         |  43 +++-
 scripts/package-offline-candidate.sh               |   4 +-
 src/cli.rs                                         |   8 +-
 src/config/mod.rs                                  |  30 ++-
 src/main.rs                                        |  15 +-
 src/persistence/storage.rs                         |  14 +-
 src/update/check.rs                                |  17 +-
 src/update/install/tests.rs                        |   6 +-
 20 files changed, 234 insertions(+), 462 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `0c15cd85258006292b894072c4c3eee67f545c43`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/prepare-cross-platform-fork-release/artifacts/post-apply.patch
```

