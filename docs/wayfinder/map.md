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

- This map only carries the durable context needed to continue the fork. The
  full research corpus (candidate evaluation, scoring, alternative-tool
  pilots) stays in the read-only research workspace referenced from
  `docs/fork/DECISIONS.md` and is not duplicated here.
- AI remains external and optional; there is no implicit model call or source
  upload.
- Never post remote review data outside an explicitly approved disposable
  target.
- One Tuicr `ReviewStore` stays canonical — never add a second store.

## Decisions so far

Full text lives in `docs/fork/DECISIONS.md`; this is only the linked gist.

- [Adopt Tuicr, maintain a small upstream-friendly fork](docs/fork/DECISIONS.md#tuicr-foundation)
  — not a greenfield build, not a composed suite of tools.
- [Keep exactly one canonical review store](docs/fork/DECISIONS.md#one-store)
  — Tuicr's `ReviewStore`, extended in place.
- [Preserve provider-native anchors behind explicit capabilities](docs/fork/DECISIONS.md#provider-native-anchors-and-capabilities)
  — Gitea and Forgejo are one adapter family with divergent profiles, pinned
  at **Gitea 1.24** / **Forgejo 16**.
- [Keep AI external and optional](docs/fork/DECISIONS.md#external-optional-ai).
- [Never silently degrade provider behavior](docs/fork/DECISIONS.md#no-silent-degradation)
  — every operation reports success/unsupported/emulated/stale/conflict/
  partial explicitly.
- [Make the fork binary unambiguously distinguishable from upstream](docs/fork/DECISIONS.md#fork-identity-update-and-data-dir-behavior)
  — distinguishable version string, disabled self-update, isolated data dir.
- [Current status is offline-only, not a release](docs/fork/DECISIONS.md#current-offline-only-status)
  — tip `ca319dc`, `OFFLINE_VALIDATED_ONLY`.

Historical patch/commit index (which `tpatch` feature produced which commits,
and what each still needs before it can go upstream):
[docs/fork/PATCHES.md](docs/fork/PATCHES.md).

## Not yet specified (fog)

- Whether upstream Tuicr will accept the durable-thread/provider-adapter
  changes, and in what split — gated on
  [upstream-and-reconcile](docs/wayfinder/tickets/upstream-and-reconcile.md).
- Real Ubuntu-under-WSL install/startup/persistence/credential behavior —
  gated on
  [validate-wsl-baseline](docs/wayfinder/tickets/validate-wsl-baseline.md).
- Live GitHub/GitLab/Azure DevOps mutation behavior against real targets —
  gated on
  [provision-provider-sandboxes](docs/wayfinder/tickets/provision-provider-sandboxes.md)
  and
  [validate-live-provider-parity](docs/wayfinder/tickets/validate-live-provider-parity.md).
- arm64 builds, signing/notarization, package-manager distribution, and the
  actual tagged/pushed release — gated on
  [release-cross-platform-fork](docs/wayfinder/tickets/release-cross-platform-fork.md).
- Minimum supported provider versions beyond the proven Gitea 1.24/Forgejo 16
  pins.
- Whether provider adapters stay in core or become feature-gated crates.
- Teaching-package update to the shipped interface (depends on
  `release-cross-platform-fork`; not tracked as its own open ticket here —
  see the research workspace's
  `docs/follow-on-map/tickets/22-update-teaching-package.md`).

## Out of scope

- Built-in model providers or source upload.
- Replacing Git, `gh`, `glab`, Azure CLI, or standard provider credentials.
- A browser UI or a second canonical review store.
- Native Windows support before macOS/WSL acceptance.
- Silent lowest-common-denominator provider behavior.
- Implementing ticket lifecycle inside `tpatch` itself, or re-litigating any
  decision already accepted in `docs/fork/DECISIONS.md`.
- Re-copying the full research corpus (candidate scoring, rejected
  alternatives, pilot transcripts) into this fork worktree — link to the
  research workspace instead.
