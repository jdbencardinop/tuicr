# Implementation Record: classify-gitlab-range-anchors

**Recorded**: 2026-08-20T03:36:15Z
**Files changed**: 1
**Patch size**: 5568 bytes
**Capture mode**: working-tree-all
**Pathspecs**: src/forge/gitlab/models.rs

## Change Summary

```
 src/forge/gitlab/models.rs | 61 ++++++++++++++++++++++++++++++++++------------
 1 file changed, 45 insertions(+), 16 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: src/forge/gitlab/models.rs
- **claim_ids**: (none)
- **base_commit**: `5d44ad197b2f02ea7152ec5ec05e864988edf7c7`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/classify-gitlab-range-anchors/artifacts/post-apply.patch
```

