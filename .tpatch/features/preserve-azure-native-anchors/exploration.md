# Exploration: preserve Azure DevOps native anchors

## Primary changes

### `src/forge/azure/backend.rs`

- Add a small conversion helper that conditionally serializes the two native
  context objects.
- Normalize the selected side's start/end into an optional
  `RemoteReviewRange`.
- Include malformed selected-side ordering in stale classification.
- Populate `RemoteReviewThread.provider_native_anchor`.

### `src/forge/azure/fixtures/threads_list.json`

- Make the current right-side fixture a real 10-12 range while retaining
  iteration and offset metadata.
- Add tracking criteria to the stale fixture so opaque retention is asserted.

### `src/forge/azure/contract_tests.rs`

- Assert normalized range, full native context retention, absent fields, and
  stale iteration payload.

### Durable model tests

- Add an Azure-shaped native payload import/re-import assertion only if the
  existing provider-neutral native-anchor tests do not already cover the
  exact persistence and no-duplication path.

## Validation

Run the focused Azure tests first, then format, clippy, and the supported
locked library suite. Record and verify the feature through `tpatch` before a
new live sandbox run.
