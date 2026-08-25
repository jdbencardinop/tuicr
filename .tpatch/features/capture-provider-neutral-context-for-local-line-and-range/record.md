# Implementation Record: capture-provider-neutral-context-for-local-line-and-range

**Recorded**: 2026-08-25T01:55:47Z
**Files changed**: 11
**Patch size**: 53915 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .tpatch/FEATURES.md                                |   1 +
 .../tickets/classify-local-anchor-shifts.md        |  35 ++-
 src/app/diff_load.rs                               |   5 +
 src/app/gaps.rs                                    |   8 +
 src/app/init.rs                                    |   1 +
 src/app/pr.rs                                      |   4 +-
 src/app/tests/thread_navigator_tests.rs            | 249 +++++++++++++++++++++
 src/app/threads.rs                                 | 179 +++++++++++----
 src/forge/context.rs                               | 143 +++++++++++-
 src/model/diff_types.rs                            | 180 +++++++++++++++
 src/model/review.rs                                | 241 +++++++++++++++++++-
 src/model/thread.rs                                |  34 +++
 12 files changed, 1035 insertions(+), 45 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `c40f55201bc606e0a5dfd97336fafa7a74bf8f62`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/capture-provider-neutral-context-for-local-line-and-range/artifacts/post-apply.patch
```

