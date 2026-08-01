# Continue the Tuicr Review Fork

**Status:** Offline implementation complete; release and upstream work
blocked on external access.

## Destination

Ship an upstream-friendly Tuicr fork that keeps one durable provider-neutral
review-thread store, publishes through capability-aware GitHub, GitLab, Azure
DevOps, Gitea, and Forgejo adapters, passes the common macOS and Ubuntu/WSL
review fixture, and upstreams what it reasonably can — without shipping a
second canonical store, a maintained greenfield product, or silent
provider-behavior degradation.

## Notes

- This map only carries the durable context needed to continue the fork.
  `docs/fork/DECISIONS.md` contains the full accepted-decision rationale;
  this map only links or gists it, never repeats or duplicates it.
- AI remains external and optional; there is no implicit model call or source
  upload.
- Never post remote review data outside an explicitly approved disposable
  target.
- One Tuicr `ReviewStore` stays canonical — never add a second store.

## Decisions so far

Full text lives in `docs/fork/DECISIONS.md`; this is only the linked gist.

- [Adopt Tuicr, maintain a small upstream-friendly fork](../fork/DECISIONS.md#tuicr-foundation)
  — not a greenfield build, not a composed suite of tools.
- [Keep exactly one canonical review store](../fork/DECISIONS.md#one-store)
  — Tuicr's `ReviewStore`, extended in place.
- [Preserve provider-native anchors behind explicit capabilities](../fork/DECISIONS.md#provider-native-anchors-and-capabilities)
  — Gitea and Forgejo are one adapter family with divergent profiles, pinned
  at **Gitea 1.24** / **Forgejo 16**.
- [Keep AI external and optional](../fork/DECISIONS.md#external-optional-ai).
- [Never silently degrade provider behavior](../fork/DECISIONS.md#no-silent-degradation)
  — every operation reports success/unsupported/emulated/stale/conflict/
  partial explicitly.
- [Make the fork binary unambiguously distinguishable from upstream](../fork/DECISIONS.md#fork-identity-update-and-data-dir-behavior)
  — distinguishable version string, disabled self-update, isolated data dir.
- [Current status is offline-only, not a release](../fork/DECISIONS.md#current-offline-only-status)
  — tip `ca319dc`, `OFFLINE_VALIDATED_ONLY`.

Historical patch/commit index (which `tpatch` feature produced which commits,
and what each still needs before it can go upstream):
[docs/fork/PATCHES.md](../fork/PATCHES.md).

## Not yet specified (fog)

- Whether upstream Tuicr will accept the durable-thread/provider-adapter
  changes, and in what split — gated on
  [upstream-and-reconcile](tickets/upstream-and-reconcile.md).
- Real Ubuntu-under-WSL install/startup/persistence/credential behavior —
  gated on
  [validate-wsl-baseline](tickets/validate-wsl-baseline.md).
- Live GitHub/GitLab/Azure DevOps mutation behavior against real targets —
  gated on
  [provision-provider-sandboxes](tickets/provision-provider-sandboxes.md)
  and
  [validate-live-provider-parity](tickets/validate-live-provider-parity.md).
- arm64 builds, signing/notarization, package-manager distribution, and the
  actual tagged/pushed release — gated on
  [release-cross-platform-fork](tickets/release-cross-platform-fork.md).
- Minimum supported provider versions beyond the proven Gitea 1.24/Forgejo 16
  pins.
- Whether provider adapters stay in core or become feature-gated crates.
- Teaching-package update to the shipped interface — depends on
  `release-cross-platform-fork` closing first; not tracked as its own open
  ticket here since it strictly follows that release.

## Out of scope

- Built-in model providers or source upload.
- Replacing Git, `gh`, `glab`, Azure CLI, or standard provider credentials.
- A browser UI or a second canonical review store.
- Native Windows support before macOS/WSL acceptance.
- Silent lowest-common-denominator provider behavior.
- Implementing ticket lifecycle inside `tpatch` itself, or re-litigating any
  decision already accepted in `docs/fork/DECISIONS.md`.
