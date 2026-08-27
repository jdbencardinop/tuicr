# Tracked Features

| Slug | Title | State | Compatibility |
|------|-------|-------|---------------|
| `capture-provider-neutral-context-for-local-line-and-range` | Capture provider-neutral context for local line and range anchors, relocate them uniquely on diff/head refresh, and mark zero/multiple matches stale or ambiguous without nearest-line guessing. | applied | unknown |
| `classify-gitlab-range-anchors` | Classify GitLab review threads as outdated when provider position head SHA differs from the current merge-request head, preserve current ranges, add regression tests, and leave the disposable live GitLab rerun explicitly blocked. | applied | unknown |
| `fix-azure-connection-data-version` | Use the official Azure DevOps Connection Data API preview version so live reviewer vote operations can resolve the authenticated viewer ID. | applied | unknown |
| `fix-github-head-refresh` | Preserve the refreshed GitHub pull-request diff and provider-relocated comments across in-process reload and fresh restart after the pull-request head advances. | applied | unknown |
| `preserve-azure-native-anchors` | Preserve Azure DevOps threadContext and pullRequestThreadContext in provider-native anchors, retain valid selected-side ranges, and persist them through durable import across head refresh. | applied | unknown |
| `wire-github-durable-publication` | Wire GitHub comment submission to the durable publication planner and checkpointed executor while preserving draft-review and legacy inline behavior. | applied | unknown |
| `wire-gitlab-durable-tui-publication` | Wire GitLab durable roots, replies, and status transitions into the TUI publication plan with resumable provider mappings and duplicate-safe retries. | applied | unknown |
