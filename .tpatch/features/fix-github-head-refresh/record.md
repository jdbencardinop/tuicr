# Implementation Record: fix-github-head-refresh

**Recorded**: 2026-08-27T04:03:44Z
**Files changed**: 5
**Patch size**: 12508 bytes
**Capture mode**: working-tree-all
**Pathspecs**: src/app/mod.rs,src/app/init.rs,src/app/commits.rs,src/app/pr.rs,src/app/tests/target_selector_tests.rs

## Change Summary

```
 src/app/commits.rs                     |   4 +
 src/app/init.rs                        |   1 +
 src/app/mod.rs                         |   3 +
 src/app/pr.rs                          |  52 +++++++++++
 src/app/tests/target_selector_tests.rs | 159 +++++++++++++++++++++++++++++++++
 5 files changed, 219 insertions(+)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: src/app/mod.rs, src/app/init.rs, src/app/commits.rs, src/app/pr.rs, src/app/tests/target_selector_tests.rs
- **claim_ids**: (none)
- **base_commit**: `6a840910b4fbcdf8dbd4ec29ee22f043318533c3`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/fix-github-head-refresh/artifacts/post-apply.patch
```

