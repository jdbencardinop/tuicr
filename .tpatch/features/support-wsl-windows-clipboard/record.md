# Implementation Record: support-wsl-windows-clipboard

**Recorded**: 2026-08-27T21:30:43Z
**Files changed**: 2
**Patch size**: 11461 bytes
**Capture mode**: working-tree-all
**Pathspecs**: CHANGELOG.md,src/output/markdown.rs

## Change Summary

```
 CHANGELOG.md           |   7 ++
 src/output/markdown.rs | 294 +++++++++++++++++++++++++++++++++++++++++++++----
 2 files changed, 281 insertions(+), 20 deletions(-)
```

## Capture Provenance

- **capture_mode**: `working-tree-all`
- **pathspecs**: CHANGELOG.md, src/output/markdown.rs
- **claim_ids**: (none)
- **base_commit**: `5da2545fa37fe05ed24b014aceac263e5b55762f`
- **upper_commit**: `working-tree`

## Replay Instructions

To re-apply this feature to a clean checkout:

```bash
# From the feature's artifacts directory:
git apply .tpatch/features/support-wsl-windows-clipboard/artifacts/post-apply.patch
```

