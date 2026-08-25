# Implementation Record: wire-gitlab-durable-tui-publication

**Recorded**: 2026-08-25T03:41:01Z
**Files changed**: 13
**Patch size**: 64720 bytes
**Capture mode**: working-tree-all

## Change Summary

```
 .tpatch/FEATURES.md                                |   1 +
 docs/wayfinder/map.md                              |   5 +-
 .../tickets/wire-gitlab-durable-publication.md     |  30 +-
 src/app/init.rs                                    |   3 +
 src/app/mod.rs                                     |  27 +
 src/app/session.rs                                 |  59 +++
 src/app/submit.rs                                  | 590 ++++++++++++++++++++-
 src/app/tests/persistence_merge_tests.rs           |  66 +++
 src/app/tests/submit_flow_tests.rs                 | 189 +++++++
 src/forge/publish.rs                               | 175 +++++-
 src/model/review.rs                                |  46 +-
 src/model/thread.rs                                |  13 +
 src/model/thread_store.rs                          |   6 +
 src/ui/submit_modals.rs                            |  49 +-
 14 files changed, 1200 insertions(+), 59 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: (none)
- **claim_ids**: (none)
- **base_commit**: `5886486963db963affdaa69fa59626e1d4980366`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/wire-gitlab-durable-tui-publication/artifacts/post-apply.patch
```

