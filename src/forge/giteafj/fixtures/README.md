# Vendored Gitea/Forgejo wire-contract fixtures

Small, sanitized excerpts of live evidence gathered against disposable local
`gitea/gitea:1.24` and `codeberg.org/forgejo/forgejo:16` containers, vendored
directly into this repository so the capability/deserialization tests and
comments in `src/forge/giteafj/`/`src/forge/capabilities.rs` can cite and load
something that actually resolves from a plain checkout of this branch —
instead of pointing at a separate, non-public research repository's
`fixtures/providers/` and `docs/findings/providers/` paths, which don't exist
here.

Every run-specific identifier (container/volume names, ephemeral base URLs,
generated user/repo names, commit SHAs) has been stripped or replaced with a
placeholder; only the provider-behavior evidence these tests/capabilities
actually depend on is kept:

- `version-gitea-1.24.7.json`, `version-forgejo-16.0.1.json`: the exact
  `GET /api/v1/version` response shape both stable pins return (Forgejo's
  version string embeds the upstream Gitea base it forked from — the
  fingerprint `verify_kind_matches` in `src/forge/giteafj/version.rs` keys
  off).
- `capability-evidence-gitea-1.24.7.json`,
  `capability-evidence-forgejo-16.0.1.json`: sanitized excerpts of the live
  probe results that back `gitea_1_24()`/`forgejo_16()` in
  `src/forge/capabilities.rs` — whether the `extra_lines_count` range
  extension is accepted/ignored, whether a second `POST` can add a comment
  to an already-pending review, and the exact 405 outcomes probed for
  reply/resolve/unresolve/edit-comment on each stable pin's live Swagger
  document.

Upstream source consulted alongside this live evidence (no live evidence
existed for behavior not exercised against the running containers above):

- <https://github.com/go-gitea/gitea/blob/main/routers/api/v1/repo/pull_review.go>
- <https://github.com/go-gitea/gitea/blob/main/modules/structs/pull_review.go>
- <https://codeberg.org/forgejo/forgejo/src/branch/forgejo/routers/api/v1/repo/pull_review.go>
- <https://codeberg.org/forgejo/forgejo/src/branch/forgejo/modules/structs/pull_review.go>
