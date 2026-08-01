# Fork Decisions

Durable, accepted decisions for this fork. Each decision lives here once;
`docs/wayfinder/map.md` links or gists it rather than repeating it. Full
rationale, rejected alternatives, and scoring evidence stay in the read-only
research workspace (`docs/decisions/`, `docs/follow-on-map/`) and are not
copied here.

## Tuicr foundation

Adopt upstream [Tuicr](https://github.com/agavra/tuicr) immediately for
local/GitHub/GitLab review, and maintain a small upstream-friendly fork
(tracked with `tpatch`) for the gaps upstream does not cover. This is a fork
of an existing MIT-licensed tool, not a greenfield build and not a suite of
composed tools (Hunk/Diffity/Neovim were evaluated and rejected as the
canonical store).

Fork branch point: upstream commit `f92502bfbbd172ceb3c16c3dfd1348b52e142be5`
(rebase to current upstream before further implementation).

Source: research workspace `docs/decisions/final-recommendation.md`.

## One store

Exactly one canonical review store: Tuicr's `ReviewStore`, extended in place.
Do not add Hunk, Diffity, or any second database as canonical review state.
Hunk's live agent API and Diffity's replies/resolution ideas may be borrowed,
but never their storage.

Source: research workspace `docs/decisions/review-artifact-contract.md`,
`docs/decisions/final-recommendation.md`.

## Provider-native anchors and capabilities

Every provider adapter (GitHub, GitLab, Azure DevOps, Gitea, Forgejo) must:

- preserve provider-native anchors/thread data losslessly in durable
  `provider_mappings`, even where the canonical model normalizes them;
- expose an explicit, versioned capability profile per host/version rather
  than one shared assumption (Gitea and Forgejo are one adapter family with
  divergent capability profiles, not one profile);
- return a typed result per operation: `success`, `unsupported` (with
  operation/provider/reason), `emulated` (with exact substitute semantics),
  `stale`/`conflict` (with current remote revision), or `partial` (with
  per-operation results).

Provider pins validated live so far: **Gitea 1.24** and **Forgejo 16**
(disposable local Docker fixtures, `fixtures/providers/`). Azure DevOps,
GitHub, and GitLab mutation behavior is implemented and offline/mock-verified
only — see "Current offline-only status" below and
`docs/wayfinder/tickets/provision-provider-sandboxes.md` /
`docs/wayfinder/tickets/validate-live-provider-parity.md`.

Source: research workspace `docs/findings/providers/provider-semantics.md`,
`docs/follow-on-map/tickets/13-implement-gitea-forgejo-adapter.md`.

## External, optional AI

AI/model access is external and optional. There is no implicit model call, no
built-in model provider, and no source upload. Humans and external agents
exchange comments only through the durable review store's stable JSON/CLI
contract.

Source: research workspace `docs/wayfinder/map.md` (Notes), `docs/decisions/review-artifact-contract.md`.

## No silent degradation

No adapter may silently omit a range, outcome, reply, or resolution, silently
guess an anchor relocation, silently move a comment to the nearest line, or
silently fall back to a lowest-common-denominator behavior across providers.
Unsupported and emulated operations must be reported as such, not hidden.

Source: research workspace `docs/decisions/review-artifact-contract.md`
("Capability behavior", "Anchor behavior").

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

Source: this worktree's `docs/offline-candidate/README.md`,
`docs/offline-candidate/SECRET-SCAN.md`, `docs/offline-candidate/MIGRATION.md`;
research workspace `retrospectives/offline-release-acceptance.md`.

## Current offline-only status

The fork is **offline-implementation-complete; release is blocked.** Current
tip: `ca319dc` (`offline-release-candidate` branch), reported version
`tuicr 0.19.1-offline-candidate.1+ca319dc`, macOS x86_64 and Linux x86_64
packages only, readiness explicitly `OFFLINE_VALIDATED_ONLY` — not a release.

Explicitly **not yet done**: real Ubuntu-under-WSL run; live GitHub/GitLab
mutation validation; live Azure DevOps sandbox validation; arm64/universal
builds; signing/notarization; package-manager distribution; any Git tag,
push, or hosted release; upstream PR submission/reconciliation.

See `docs/handoff/CURRENT.md` for the exact frontier and
`docs/wayfinder/tickets/` for the open/blocking tickets that gate a real
release.

Source: research workspace `retrospectives/offline-release-acceptance.md`,
`docs/follow-on-map/map.md`.
