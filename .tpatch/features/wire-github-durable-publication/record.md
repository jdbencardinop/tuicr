# Implementation Record: wire-github-durable-publication

**Recorded**: 2026-08-27T03:10:05Z
**Files changed**: 10
**Patch size**: 41568 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .tpatch/FEATURES.md                                |   1 +
 docs/handoff/CURRENT.md                            |  13 +-
 docs/wayfinder/map.md                              |   5 +-
 .../tickets/wire-github-durable-publication.md     |  19 ++-
 src/app/mod.rs                                     |   2 +-
 src/app/submit.rs                                  | 131 +++++++++++--------
 src/app/tests/submit_flow_tests.rs                 | 126 ++++++++++++-------
 src/forge/github/gh.rs                             |  50 ++++++--
 src/forge/github/mutations.rs                      | 139 +++++++++++++++------
 src/forge/publish.rs                               |   8 +-
 src/ui/submit_modals.rs                            |   6 +-
 11 files changed, 341 insertions(+), 159 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `45f8e7cf2a2daf392cf97b4685ae8961dccf0ced`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/wire-github-durable-publication/artifacts/post-apply.patch
```

