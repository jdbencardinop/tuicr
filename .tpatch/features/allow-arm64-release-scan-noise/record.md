# Implementation Record: allow-arm64-release-scan-noise

**Recorded**: 2026-08-29T01:53:31Z
**Files changed**: 2
**Patch size**: 3415 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .tpatch/FEATURES.md                 |  1 +
 scripts/package-release-artifact.sh | 10 ++++++++--
 2 files changed, 9 insertions(+), 2 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `720e4a9adbbf29b49bb9b535619e9d1016eac7bb`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/allow-arm64-release-scan-noise/artifacts/post-apply.patch
```

