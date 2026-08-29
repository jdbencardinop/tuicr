# Implementation Record: defer-nix-release-channel

**Recorded**: 2026-08-29T01:59:39Z
**Files changed**: 4
**Patch size**: 3248 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .github/workflows/build_nix.yml | 8 ++++----
 .tpatch/FEATURES.md             | 1 +
 README.md                       | 5 ++---
 RELEASE.md                      | 5 +++--
 docs/release/INSTALL.md         | 5 +++--
 5 files changed, 13 insertions(+), 11 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `e7ccb8f180a0a0e76bea56fe04a2904c7faf6034`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/defer-nix-release-channel/artifacts/post-apply.patch
```

