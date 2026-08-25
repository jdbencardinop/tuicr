# Exploration: capture provider-neutral context for local anchors

## Primary changes

### `src/model/diff_types.rs`

- Add side-aware extraction of a bounded `AnchorContext` from displayed diff
  lines.
- Require the complete selected line/range to exist on the requested side.
- Include only contiguous numbered neighbors, preventing unrelated hunks from
  becoming false context.
- Test old/new separation, ranges, and missing selection.

### `src/model/thread.rs`

- Add an API that attaches context only to line/range anchors and validates
  selected length against the target span.
- Expose whether an anchor has relocation context so refresh can skip
  untrusted legacy anchors.
- Keep file/review anchors unchanged and retain all existing relocation rules.

### `src/app/threads.rs`

- Snapshot durable thread IDs before legacy mirroring.
- After migration, attach displayed-side context only to newly created
  line/range threads.
- Generalize content collection/refresh for local reload while preserving the
  PR head-advance path.
- After refresh, synchronize current durable positions into legacy shadows.

### `src/model/review.rs`

- Add deterministic legacy-shadow synchronization keyed by durable thread
  comment IDs.
- Re-key only current line/range anchors in the matching file.
- Update side and range metadata together; leave stale, ambiguous, file, and
  review anchors untouched.

### `src/app/diff_load.rs`

- Invoke the generalized local anchor refresh after replacement diffs are
  loaded and before annotations are rebuilt.
- Surface refresh errors through the existing App error path rather than
  swallowing them.

## Tests

- Domain capture:
  - new-side line/range context;
  - old-side deletion context;
  - no cross-side borrowing;
  - contiguous-neighbor boundaries.
- App/migration:
  - only newly mirrored threads gain context;
  - cold legacy migration remains context-free;
  - repeat mirroring remains idempotent.
- Reload:
  - line 42 relocates to 47 after five inserted lines;
  - range span relocates intact;
  - missing context becomes stale;
  - duplicate context becomes ambiguous;
  - current relocated shadows move to the canonical terminal line;
  - stale/ambiguous shadows remain at their last row with status;
  - file anchors remain unchanged.
- Rendering/annotations:
  - unified and side-by-side annotation rows follow the re-keyed legacy
    shadow.

## Invariants

- Durable threads are canonical; legacy comments remain compatibility shadows.
- Context is captured at creation, never reconstructed later from an old line.
- Old/new side identity is preserved.
- Only one exact match moves an anchor.
- Zero/multiple matches never move a legacy shadow.
- Closed threads remain frozen.
- No network calls or provider mutations are introduced.

## Validation

Run formatting, focused model/App/UI tests, all-target clippy with warnings
denied, and the supported locked library suite. Then perform the documented
WSL fixture/TUI reload, retain raw local output only under ignored
`artifacts/raw/`, record the patch, and run `tpatch test` and `tpatch verify`
before landing.
