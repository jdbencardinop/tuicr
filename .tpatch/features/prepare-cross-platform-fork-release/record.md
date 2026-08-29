# Implementation Record: prepare-cross-platform-fork-release

**Recorded**: 2026-08-29T01:36:05Z
**Files changed**: 1
**Patch size**: 389 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .github/workflows/release-candidate.yml | 2 ++
 1 file changed, 2 insertions(+)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `42a9597e2dd361ad66943c4a177e10ab3efd2d3e`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/prepare-cross-platform-fork-release/artifacts/post-apply.patch
```

