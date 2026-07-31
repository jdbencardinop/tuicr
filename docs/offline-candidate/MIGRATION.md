# Migration, backup, and rollback — offline-candidate build

This document is about the **fork's session-store data**, not your Git
repositories. tuicr never modifies anything inside a reviewed repo except
through explicit provider-publish operations you invoke.

## Data location

Review session JSON files live under a per-OS data directory (resolved via
the `directories` crate's `ProjectDirs::from("", "", "tuicr")`, distinct from
its XDG-based config directory below):

| OS | Reviews data directory |
| --- | --- |
| macOS | `~/Library/Application Support/tuicr/reviews/` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/tuicr/reviews/` |

Config (theme selection, `config.toml`, custom themes) lives separately,
uniformly under XDG on both platforms:

| OS | Config directory |
| --- | --- |
| macOS / Linux | `${XDG_CONFIG_HOME:-~/.config}/tuicr/` |

A storage-manifest lock file (`.tuicr.lock`) inside the reviews directory
coordinates concurrent access; don't hand-edit it.

## This shares its data directory with any real upstream `tuicr` install

The reviews directory above is resolved via
`ProjectDirs::from("", "", "tuicr")` — a literal `"tuicr"` app identifier
that this fork does not change. **If you also have real upstream `tuicr`
installed on the same machine, this fork binary reads and writes the exact
same `reviews/` directory as that upstream install** — there is no
separate fork-specific data directory. Both binaries will see (and can
race on) each other's sessions. If you need to keep the fork's test data
isolated from a real upstream install, override `XDG_DATA_HOME` (Linux) or
set `HOME` to a scratch directory before running this binary, or back up
and clear `reviews/` before switching between the two installs.

## Back up before migrating or rolling back

Always back up the reviews directory before switching binaries in either
direction:

```bash
# macOS
cp -R "$HOME/Library/Application Support/tuicr/reviews" \
      "$HOME/Library/Application Support/tuicr/reviews.bak-$(date +%Y%m%d)"

# Linux
cp -R "${XDG_DATA_HOME:-$HOME/.local/share}/tuicr/reviews" \
      "${XDG_DATA_HOME:-$HOME/.local/share}/tuicr/reviews.bak-$(date +%Y%m%d)"
```

This is cheap (it's just JSON) and is the only real safety net described
below.

## Session schema version 1.4

This fork's sessions are written at `CURRENT_SESSION_VERSION = "1.4"`
(`src/model/thread_store.rs`). Sessions below that version have their legacy
per-file/per-line `Comment`s migrated into durable, provider-neutral
`PersistedThread`s the first time they load — non-destructively: legacy
comment fields are preserved, never removed, so older tooling that only
understands the legacy fields keeps working. Opening a pre-1.4 session with
this build is safe and one-directional (it upgrades in place on next save).

## Rolling back to upstream 0.19.1

This fork is based on unmodified upstream Tuicr `0.19.1` and does not bump
`Cargo.toml`'s version, so a rollback means: stop using this fork's binary
and install real upstream `0.19.1` instead (e.g. via `cargo install
tuicr@0.19.1`, Homebrew, or a GitHub release binary).

**This is a lossy operation for any session that has fork-only data.**
Upstream `0.19.1`'s `ReviewSession` struct has no `threads` field. When
upstream loads a session JSON written by this fork:

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

**Required rollback procedure:**

1. Back up the reviews directory (see above) before installing upstream.
2. Install upstream `0.19.1`.
3. Treat every session with the newer schema/threads as read-only under
   upstream. If you must keep editing a session under upstream, copy your
   backup aside first — do not rely on upstream's own save path to preserve
   fork-only data.
4. To return to the fork later, restore from the backup rather than
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
