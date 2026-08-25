# Analysis: preserve Azure DevOps native anchors

## Observed behavior

The 2026-08-25 live run returned both `threadContext` and
`pullRequestThreadContext` after a synthetic head update. The adapter used the
iteration pair to mark the thread outdated, but emitted
`provider_native_anchor: None` and `range: None`.

`thread_provider_mapping` already proves both context DTOs serialize cleanly.
`thread_from_remote` already persists any `provider_native_anchor` under the
provider mapping's `native_anchor` key and imports a validated normalized
range. The loss therefore occurs only in
`AzureDevOpsBackend::list_review_threads`.

## Compatibility and risk

- Azure can populate left and right positions simultaneously. Existing
  behavior selects right when present; that precedence must remain.
- Start/end offsets and the non-selected side cannot fit the normalized
  provider-neutral anchor and must remain in the opaque native payload.
- A selected-side end before its start is malformed provider data. It must
  mark the thread outdated without swapping endpoints or inventing a range.
- Single-line anchors must remain line anchors rather than becoming
  one-element range anchors.
- General threads have no file anchor. A pull-request thread context without
  a file context must still be preserved if Azure supplies one.

## Recommendation

Serialize present Azure context objects into one opaque native-anchor object
before consuming the thread. Derive an optional normalized range only from
the already-selected side when its end is strictly after its start. Treat a
reversed selected-side range as stale evidence. Reuse the existing generic
durable import and mapping merge paths.
