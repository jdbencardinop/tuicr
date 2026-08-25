# Specification: fix Azure Connection Data API version

## Goal

Allow Azure DevOps vote operations to resolve the authenticated viewer ID
against the documented cloud API.

## Acceptance criteria

1. `viewer_id` requests Connection Data with `api-version=7.1-preview.1`.
2. All other Azure DevOps Git endpoints remain pinned to stable version 7.1.
3. Existing approve, request-changes, and direct vote contract tests assert
   the preview Connection Data request and pass.
4. The focused Azure adapter test suite passes.
5. The approved live probe advances past viewer lookup.

## Non-goals

- Changing vote semantics.
- Making draft pull requests votable.
- Modifying authentication or credential storage.
- Fixing the separately observed provider-native anchor retention gap.
