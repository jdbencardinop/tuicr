# Migration, backup, and rollback — offline-candidate build

This document is about the **fork's session-store data**, not your Git
repositories. tuicr never modifies anything inside a reviewed repo except
through explicit provider-publish operations you invoke.

## Data location

Review session JSON files live under a per-OS data directory (resolved via
the `directories` crate's
`ProjectDirs::from("", "", "tuicr-offline-candidate")`, distinct from its
XDG-based config directory below). This fork-specific app id is
deliberately **different** from upstream `tuicr`'s own `"tuicr"` app id —
see the next section.

| OS | Reviews data directory |
| --- | --- |
| macOS | `~/Library/Application Support/tuicr-offline-candidate/reviews/` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/tuicr-offline-candidate/reviews/` |

Config (theme selection, `config.toml`, custom themes) lives separately,
uniformly under XDG on both platforms:

| OS | Config directory |
| --- | --- |
| macOS / Linux | `${XDG_CONFIG_HOME:-~/.config}/tuicr-offline-candidate/` |

A storage-manifest lock file (`.tuicr.lock`) inside the reviews directory
coordinates concurrent access; don't hand-edit it.

## No collision with a real upstream `tuicr` install

This build resolves its data/config directories under the fork-specific app
id `tuicr-offline-candidate`, not upstream's `tuicr`. **A real upstream
`tuicr` install on the same machine reads/writes its own separate
`~/Library/Application Support/tuicr/reviews/` (macOS) or
`${XDG_DATA_HOME:-~/.local/share}/tuicr/reviews/` (Linux) directory — this
fork build never touches it, and vice versa.** There is no shared-directory
race between the two anymore.

If you have existing review sessions under a real upstream `tuicr` install
that you want available in this fork build, use the bundled one-way import
script rather than copying manually — it backs up the fork's destination
directory first and never touches (moves/deletes) the upstream source:

```bash
./import-upstream-reviews.sh --dry-run   # preview, copies nothing
./import-upstream-reviews.sh             # perform the one-way copy
```

(`import-upstream-reviews.sh` is bundled at the root of this archive, and
also lives at `scripts/import-upstream-reviews.sh` in the fork's source
tree. It auto-detects both directories per OS; pass `--from`/`--to` to
override.)

## Back up before migrating or rolling back

Always back up the reviews directory before switching binaries in either
direction:

```bash
# macOS (this fork's data)
cp -R "$HOME/Library/Application Support/tuicr-offline-candidate/reviews" \
      "$HOME/Library/Application Support/tuicr-offline-candidate/reviews.bak-$(date +%Y%m%d)"

# Linux (this fork's data)
cp -R "${XDG_DATA_HOME:-$HOME/.local/share}/tuicr-offline-candidate/reviews" \
      "${XDG_DATA_HOME:-$HOME/.local/share}/tuicr-offline-candidate/reviews.bak-$(date +%Y%m%d)"
```

This is cheap (it's just JSON) and is the only real safety net described
below. `scripts/import-upstream-reviews.sh` performs this backup step for
you automatically before any import.


## Session schema version 1.4

This fork's sessions are written at `CURRENT_SESSION_VERSION = "1.4"`
(`src/model/thread_store.rs`). Sessions below that version have their legacy
per-file/per-line `Comment`s migrated into durable, provider-neutral
`PersistedThread`s the first time they load — non-destructively: legacy
comment fields are preserved, never removed, so older tooling that only
understands the legacy fields keeps working. Opening a pre-1.4 session with
this build is safe and one-directional (it upgrades in place on next save).

## Rolling back to upstream 0.19.1

This fork is based on upstream Tuicr `0.19.1`. Rolling back means: stop
using this fork's binary and install real upstream `0.19.1` instead (e.g.
via `cargo install tuicr@0.19.1`, Homebrew, or a GitHub release binary).
Because this build's data directory (`tuicr-offline-candidate`) is now
separate from upstream's (`tuicr`), installing/running upstream **does
not, by itself, touch this fork's session data at all** — upstream simply
won't see it (it looks in its own `tuicr` directory). Rollback only
matters if you want upstream to see/continue sessions this fork created,
which requires an explicit copy in the upstream direction (the reverse of
`scripts/import-upstream-reviews.sh`, which only copies upstream-to-fork;
copy `reviews/` from the fork's directory into upstream's manually if
needed, backing up upstream's directory first).

**Copying fork sessions to upstream is a lossy operation for any session
that has fork-only data.** Upstream `0.19.1`'s `ReviewSession` struct has no
`threads` field. When upstream loads a session JSON written by this fork:

- Deserialization succeeds — `serde` silently ignores the unrecognized
  `threads` array (upstream's struct has no field to put it in) and any
  other fork-only fields.
- If upstream then **saves** that session again (e.g. after adding one more
  comment through its own legacy comment path), the rewritten JSON **will
  not contain the `threads` array anymore** — durable thread-only state
  (thread IDs, provider-neutral anchors not mirrored into a legacy comment,
  resolution/reply state recorded only on a thread) is silently dropped from
  disk at that point, not just from memory.
- Legacy `review_comments`/`files[..].*_comments` data is unaffected; only
  state that exists *exclusively* on `threads` is at risk.

**Required rollback procedure (only if you need upstream to see this
fork's sessions):**

1. Back up the fork's reviews directory (see above).
2. Install upstream `0.19.1`.
3. Copy the specific session file(s) you need into upstream's `tuicr`
   reviews directory (back that up first too, since it's a distinct
   directory upstream already owns).
4. Treat every copied session with the newer schema/threads as read-only
   under upstream. If you must keep editing it under upstream, copy your
   backup aside first — do not rely on upstream's own save path to preserve
   fork-only data.
5. To return to the fork later, restore the fork's own backup rather than
   whatever upstream last wrote, if you want thread-only state back.


## No live-provider or WSL validation in this build

This archive's Azure DevOps and Gitea/Forgejo support is verified only
against official API documentation and recorded fixtures (offline). GitHub
and GitLab read paths have prior live evidence from earlier evaluation work,
but no external write was performed as part of producing this archive. The
Linux binary here was built and smoke-tested in a pinned Docker container,
which is **not** a substitute for validating actual Ubuntu-under-WSL
filesystem/clipboard/terminal behavior. Treat both as open evidence gaps,
not passed checks — see `docs/follow-on-map/tickets/03-validate-wsl-baseline.md`
and `docs/follow-on-map/tickets/04-provision-provider-sandboxes.md` in the
research workspace.

## Uninstall

See `INSTALL.md`'s uninstall section. Uninstalling removes the binary and,
if you choose, the data/config directories above — it never touches a
reviewed Git repository or any remote provider account.
