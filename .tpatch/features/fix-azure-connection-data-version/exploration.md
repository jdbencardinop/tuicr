# Exploration: fix Azure Connection Data API version

## Changes

- `src/forge/azure/backend.rs`
  - Build the Connection Data URL with the documented
    `7.1-preview.1` version in `viewer_id`.
- `src/forge/azure/contract_tests.rs`
  - Update the three exact Connection Data request expectations.

## Validation

Run:

```text
cargo test --locked --lib forge::azure::contract_tests
cargo test --locked --lib forge::azure
```

The live rerun is evidence only and remains outside the committed test suite.
