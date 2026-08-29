#!/usr/bin/env bash
set -euo pipefail

SOURCE="upstream"
DRY_RUN=0
FORCE=0
FROM_DIR=""
TO_DIR=""

usage() {
  cat <<'USAGE'
Usage: scripts/import-reviews.sh [options]

Copy review data into the maintained fork without modifying the source.

Options:
  --source NAME  Source layout: upstream (default) or offline-candidate.
  --from DIR     Override the source reviews directory.
  --to DIR       Override the fork reviews directory.
  --force        Permit a non-empty destination after backing it up.
  --dry-run      Print the resolved operation without copying.
  -h, --help     Show this help.
USAGE
}

log() { printf '[import-reviews] %s\n' "$*" >&2; }
die() { printf '[import-reviews] ERROR: %s\n' "$*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source) SOURCE="${2:?--source requires a value}"; shift 2 ;;
    --from) FROM_DIR="${2:?--from requires a value}"; shift 2 ;;
    --to) TO_DIR="${2:?--to requires a value}"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

case "$SOURCE" in
  upstream) source_app_id="tuicr" ;;
  offline-candidate) source_app_id="tuicr-offline-candidate" ;;
  *) die "--source must be upstream or offline-candidate" ;;
esac

default_data_home() {
  case "$(uname -s)" in
    Darwin) printf '%s/Library/Application Support' "$HOME" ;;
    *) printf '%s' "${XDG_DATA_HOME:-$HOME/.local/share}" ;;
  esac
}

data_home="$(default_data_home)"
FROM_DIR="${FROM_DIR:-$data_home/$source_app_id/reviews}"
TO_DIR="${TO_DIR:-$data_home/tuicr-fork/reviews}"

[[ -d "$FROM_DIR" ]] || die "source directory does not exist: $FROM_DIR"
source_count="$(find "$FROM_DIR" -type f | wc -l | tr -d ' ')"
destination_count=0
if [[ -e "$TO_DIR" ]]; then
  destination_count="$(find "$TO_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')"
fi

log "source ($SOURCE): $FROM_DIR ($source_count file(s))"
log "destination: $TO_DIR ($destination_count file(s))"

if [[ "$destination_count" -gt 0 && "$FORCE" -ne 1 ]]; then
  die "destination is non-empty; rerun with --force to back it up before import"
fi

if [[ "$DRY_RUN" -eq 1 ]]; then
  log "[dry-run] would back up the destination if present, then copy source to destination"
  exit 0
fi

if [[ -e "$TO_DIR" ]]; then
  backup_dir="${TO_DIR}.bak-$(date -u +%Y%m%dT%H%M%SZ)"
  log "backing up destination to: $backup_dir"
  cp -R "$TO_DIR" "$backup_dir"
fi

mkdir -p "$TO_DIR"
cp -R "$FROM_DIR/." "$TO_DIR/"
copied_count="$(find "$TO_DIR" -type f | wc -l | tr -d ' ')"
log "import complete: $copied_count destination file(s); source was not modified"
