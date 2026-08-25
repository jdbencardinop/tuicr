# Implementation Record: fix-azure-connection-data-version

**Recorded**: 2026-08-25T19:41:56Z
**Files changed**: 2
**Patch size**: 2421 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .tpatch/FEATURES.md               | 1 +
 src/forge/azure/backend.rs        | 6 +++---
 src/forge/azure/contract_tests.rs | 6 +++---
 3 files changed, 7 insertions(+), 6 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `522824eaf3e74113109ed47bca89af2b914213ea`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/fix-azure-connection-data-version/artifacts/post-apply.patch
```

