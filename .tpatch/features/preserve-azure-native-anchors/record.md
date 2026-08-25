# Implementation Record: preserve-azure-native-anchors

**Recorded**: 2026-08-25T23:49:20Z
**Files changed**: 6
**Patch size**: 30327 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .tpatch/FEATURES.md                                |   1 +
 .../tickets/preserve-azure-native-anchors.md       |   4 +-
 src/forge/azure/backend.rs                         | 122 +++++++++++++---
 src/forge/azure/contract_tests.rs                  | 100 ++++++++++++-
 src/forge/azure/fixtures/threads_list.json         | 102 ++++++++++++-
 src/forge/azure/live_tests.rs                      | 160 ++++++++++++++++++++-
 src/forge/azure/models.rs                          |  40 ++++--
 7 files changed, 480 insertions(+), 49 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `44f557654d97fcb9d1fef757705512b6d7f80448`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/preserve-azure-native-anchors/artifacts/post-apply.patch
```

