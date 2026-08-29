# Continue the Tuicr Review Fork

**Status:** Implementation and live provider validation complete;
cross-platform release engineering is the active frontier.

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
- [Prepare a fork-owned release channel](tickets/release-cross-platform-fork.md#release-design)
  — fork identity, isolated storage, native four-platform candidate CI, and
  guarded draft-prerelease promotion are implemented; all native targets pass
  and tag/draft promotion awaits explicit authorization.
- [Validate upstream and fork on Ubuntu under WSL](tickets/validate-wsl-baseline.md)
  — core build/startup/navigation/persistence/stdout/editor and read-only
  provider checks passed; the two newer-Git fixtures and browser launcher
  remain explicit environment caveats.
- [Validate live provider parity](tickets/validate-live-provider-parity.md)
  — GitHub, GitLab, and Azure DevOps mutation lifecycles live-pass; accepted
  identity and Azure pagination limits remain explicit evidence gaps.
- [Classify GitLab range anchors](tickets/classify-gitlab-range-anchors.md)
  — live GitLab 19.2.1 now preserves native range 70-72 and marks either
  old-head or provider-terminal mismatches stale through backend, TUI reload,
  and durable storage.
- [Relocate local anchors safely](tickets/classify-local-anchor-shifts.md)
  — TUI-created line/range anchors capture exact-side context, relocate only
  on one exact match, persist legacy shadows at the canonical row, and expose
  zero/multiple matches as stale/ambiguous.
- [Publish durable GitLab threads from the TUI](tickets/wire-gitlab-durable-publication.md)
  — `:submit comment` checkpoints roots, replies, resolve, and reopen;
  provider-ID reconciliation and retry planning prevent duplicates.
- [Publish durable GitHub threads from the TUI](tickets/wire-github-durable-publication.md)
  — live root/reply/resolve/reopen publication and checkpointed restart retry
  pass without duplicates.
- [Preserve GitHub discussions across head refresh](tickets/fix-github-head-refresh.md)
  — automatic since-last-review scoping now yields to the cumulative diff
  when it would hide an active relocated provider thread.
- [Write WSL exports to Windows Clipboard](tickets/decide-wsl-clipboard-boundary.md)
  — active WSL interop now uses a UTF-16LE `clip.exe` bridge before terminal
  and Linux clipboard fallbacks without changing native platform routing.
- [Provision provider sandboxes](tickets/provision-provider-sandboxes.md) —
  Disposable GitHub, GitLab, and Azure DevOps targets were approved,
  exercised with synthetic-only data, and fully torn down.
- [Fix Azure Connection Data version](tickets/fix-azure-connection-data-version.md)
  — The documented `7.1-preview.1` endpoint now resolves the live viewer ID;
  all Git endpoints remain on stable 7.1.
- [Preserve Azure native anchors](tickets/preserve-azure-native-anchors.md)
  — live adapter, ReviewStore, and TUI retain shifted iteration context and
  expose the thread as outdated without guessing relocation.

Historical patch/commit index (which `tpatch` feature produced which commits,
and what each still needs before it can go upstream):
[docs/fork/PATCHES.md](../fork/PATCHES.md).

## Not yet specified (fog)

- Whether upstream Tuicr will accept the durable-thread/provider-adapter
  changes, and in what split — gated on
  [upstream-and-reconcile](tickets/upstream-and-reconcile.md).
- Native self-hosted GitLab custom-port URLs without the documented portless
  logical-host workaround — tracked by
  [support-gitlab-custom-ports](tickets/support-gitlab-custom-ports.md).
- Developer ID signing/notarization disposition and the actual tagged/pushed
  release —
  gated on
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
