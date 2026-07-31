#!/usr/bin/env bash
# Explicit, one-way COPY of review-session data from a real upstream `tuicr`
# install's data directory into this offline-candidate fork's own
# fork-specific data directory (`tuicr-offline-candidate`, distinct from
# upstream's `tuicr` -- see docs/offline-candidate/MIGRATION.md).
#
# This script:
#   - never touches the upstream source directory (copy only, no move/rm);
#   - never auto-overwrites an existing destination -- it backs it up first
#     and requires --force to proceed if the destination is non-empty;
#   - is read-only with --dry-run;
#   - does not talk to any network or provider.
#
# IMPORTANT: older upstream tuicr builds (pre-thread-support) can drop
# thread-only state on save. If you plan to round-trip data back to
# upstream, read the "rollback to upstream" section in
# docs/offline-candidate/MIGRATION.md first.
set -euo pipefail

DRY_RUN=0
FORCE=0
FROM_DIR=""
TO_DIR=""

usage() {
  cat <<'USAGE' >&2
Usage: scripts/import-upstream-reviews.sh [--from DIR] [--to DIR] [--force] [--dry-run]

Options:
  --from DIR   Upstream tuicr reviews directory to copy FROM.
               Default (auto-detected per OS):
                 macOS:  ~/Library/Application Support/tuicr/reviews
                 Linux:  ${XDG_DATA_HOME:-~/.local/share}/tuicr/reviews
  --to DIR     This fork's reviews directory to copy TO.
               Default (auto-detected per OS):
                 macOS:  ~/Library/Application Support/tuicr-offline-candidate/reviews
                 Linux:  ${XDG_DATA_HOME:-~/.local/share}/tuicr-offline-candidate/reviews
  --force      Proceed even if --to already exists and is non-empty. A
               timestamped backup of --to is still always taken first.
  --dry-run    Print what would happen; copy nothing.
  -h, --help   Show this help.

This is a one-way COPY. The upstream source directory is never modified,
moved, or deleted. If --to already exists, it is backed up (renamed with a
timestamp suffix) before anything is written to it.
USAGE
}

log() { printf '[import-upstream-reviews] %s\n' "$*" >&2; }
die() { printf '[import-upstream-reviews] ERROR: %s\n' "$*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --from) FROM_DIR="${2:?--from requires a value}"; shift 2 ;;
    --to) TO_DIR="${2:?--to requires a value}"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

default_data_home() {
  case "$(uname -s)" in
    Darwin) printf '%s/Library/Application Support' "$HOME" ;;
    *) printf '%s' "${XDG_DATA_HOME:-$HOME/.local/share}" ;;
  esac
}

if [[ -z "$FROM_DIR" ]]; then
  FROM_DIR="$(default_data_home)/tuicr/reviews"
  log "auto-detected upstream source: $FROM_DIR"
fi
if [[ -z "$TO_DIR" ]]; then
  TO_DIR="$(default_data_home)/tuicr-offline-candidate/reviews"
  log "auto-detected fork destination: $TO_DIR"
fi

[[ -d "$FROM_DIR" ]] || die "upstream source directory does not exist: $FROM_DIR (nothing to import)"

FROM_COUNT="$(find "$FROM_DIR" -type f | wc -l | tr -d ' ')"
log "source ($FROM_DIR): $FROM_COUNT file(s)"

if [[ -e "$TO_DIR" ]]; then
  TO_COUNT="$(find "$TO_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')"
  if [[ "$TO_COUNT" -gt 0 && "$FORCE" -ne 1 ]]; then
    die "destination already exists and is non-empty ($TO_COUNT file(s)) at $TO_DIR; re-run with --force to proceed (a backup will still be taken first), or back it up/remove it yourself"
  fi
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  log "[dry-run] would back up '$TO_DIR' (if present) then copy '$FROM_DIR' -> '$TO_DIR'"
  log "[dry-run] no files were copied, moved, or deleted"
  exit 0
fi

if [[ -e "$TO_DIR" ]]; then
  BACKUP_DIR="${TO_DIR}.bak-$(date -u +%Y%m%dT%H%M%SZ)"
  log "backing up existing destination to: $BACKUP_DIR"
  cp -R "$TO_DIR" "$BACKUP_DIR"
fi

mkdir -p "$TO_DIR"
log "copying (one-way, source untouched): $FROM_DIR -> $TO_DIR"
cp -R "$FROM_DIR/." "$TO_DIR/"

TO_COUNT_AFTER="$(find "$TO_DIR" -type f | wc -l | tr -d ' ')"
log "done. destination now has $TO_COUNT_AFTER file(s). Source directory was not modified."
log "Reminder: older upstream tuicr builds can drop thread-only state on save;"
log "if you later roll back to upstream, back up '$TO_DIR' again first."
