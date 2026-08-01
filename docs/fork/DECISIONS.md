# Fork Decisions

Durable, accepted decisions for this fork. Each decision lives here once,
with enough rationale to stand on its own; `docs/wayfinder/map.md` only links
or gists it rather than repeating it. This file does not depend on any
external research corpus — every claim below is either self-contained or
points to a file/URL inside this repo or a public upstream source.

## Tuicr foundation

Adopt upstream [Tuicr](https://github.com/agavra/tuicr) immediately for
local/GitHub/GitLab review, and maintain a small upstream-friendly fork
(tracked with `tpatch`) for the gaps upstream does not cover. This is a fork
of an existing MIT-licensed tool, not a greenfield build and not a suite of
composed tools: Hunk (live-only, no durable store), Diffity (license
contradiction between its stated and bundled terms), and Neovim/codediff
companions (narrower surface, not a full review-authoring tool) were each
evaluated and rejected as the canonical store, kept at most as context
companions.

Fork branch point: upstream commit `f92502bfbbd172ceb3c16c3dfd1348b52e142be5`
(rebase to current upstream before further implementation).

## One store

Exactly one canonical review store: Tuicr's `ReviewStore`, extended in place.
Do not add Hunk, Diffity, or any second database as canonical review state.
Splitting review state across stores would break cross-tool consistency (two
sources of truth for the same comment/thread) and double the migration and
provider-sync surface. Hunk's live agent API and Diffity's replies/resolution
ideas may be borrowed as design inspiration, but never their storage.

## Provider-native anchors and capabilities

Every provider adapter (GitHub, GitLab, Azure DevOps, Gitea, Forgejo) must:

- preserve provider-native anchors/thread data losslessly in durable
  `provider_mappings`, even where the canonical model normalizes them, so no
  round-trip to a provider ever loses information the provider itself
  tracks;
- expose an explicit, versioned capability profile per host/version rather
  than one shared assumption (Gitea and Forgejo are one adapter family with
  divergent capability profiles, not one profile — e.g. reply/resolve/edit
  routes are unsupported on both at the versions pinned below, confirmed via
  405 responses against their stable Swagger definitions);
- return a typed result per operation: `success`, `unsupported` (with
  operation/provider/reason), `emulated` (with exact substitute semantics),
  `stale`/`conflict` (with current remote revision), or `partial` (with
  per-operation results) — so a caller never has to guess whether an
  operation silently no-op'd.

Provider pins validated live so far: **Gitea 1.24** and **Forgejo 16**
(disposable local Docker fixtures). Azure DevOps, GitHub, and GitLab mutation
behavior is implemented and offline/mock-verified only — see "Current
offline-only status" below and
`docs/wayfinder/tickets/provision-provider-sandboxes.md` /
`docs/wayfinder/tickets/validate-live-provider-parity.md`.

## External, optional AI

AI/model access is external and optional. There is no implicit model call, no
built-in model provider, and no source upload. Humans and external agents
exchange comments only through the durable review store's stable JSON/CLI
contract — this keeps the review store useful with zero AI configured, and
never forces diff/comment content through a third-party model call a user
did not explicitly invoke.

## No silent degradation

No adapter may silently omit a range, outcome, reply, or resolution, silently
guess an anchor relocation, silently move a comment to the nearest line, or
silently fall back to a lowest-common-denominator behavior across providers.
Unsupported and emulated operations must be reported as such (see the typed
result list above), not hidden — a reviewer or agent acting on an
unreported partial failure is worse than one that is told to expect it.

## Fork identity, update, and data-dir behavior

The packaged fork binary must be unambiguously distinguishable from upstream
and must never silently self-replace with upstream:

- `--version` reports a distinguishable prerelease tag plus the embedded
  source commit SHA (e.g. `tuicr 0.19.1-offline-candidate.N+<sha>`), never a
  plain upstream version string.
- The automatic background update check and `tuicr update` are disabled at
  the source level (not just by config flag) and make zero network calls;
  attempting `tuicr update` fails immediately with an explicit
  fork-is-disabled message.
- Fork config/data directories are isolated from upstream's, with a
  backup-first, one-way import/migration helper for existing upstream data.
- Packaging includes a byte-aware, fail-closed secret scan of both source and
  compiled binaries before any archive is produced.

These four rules exist together so that installing, running, or updating the
fork can never silently overwrite, delete, or leak into an existing upstream
Tuicr install. See this repo's `docs/offline-candidate/README.md`,
`docs/offline-candidate/SECRET-SCAN.md`, and
`docs/offline-candidate/MIGRATION.md` for the exact mechanics.

## Current offline-only status

The fork is **offline-implementation-complete; release is blocked.** Current
tip: `ca319dc` (implementation/package base), reported version
`tuicr 0.19.1-offline-candidate.1+ca319dc`, macOS x86_64 and Linux x86_64
packages only, readiness explicitly `OFFLINE_VALIDATED_ONLY` — not a release.

Explicitly **not yet done**: real Ubuntu-under-WSL run; live GitHub/GitLab
mutation validation; live Azure DevOps sandbox validation; arm64/universal
builds; signing/notarization; package-manager distribution; any Git tag,
push, or hosted release; upstream PR submission/reconciliation.

See `docs/handoff/CURRENT.md` for the exact frontier and
`docs/wayfinder/tickets/` for the open/blocking tickets that gate a real
release.
