# Fork data migration and rollback

The maintained fork stores reviews under `tuicr-fork`, separate from both
upstream `tuicr` and the archived `tuicr-offline-candidate` build.

| Platform | Reviews |
| --- | --- |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/tuicr-fork/reviews/` |
| macOS | `~/Library/Application Support/tuicr-fork/reviews/` |

Config and themes use `${XDG_CONFIG_HOME:-~/.config}/tuicr-fork/`.

Import is explicit, one-way, and backup-first:

```bash
./import-reviews.sh --source upstream --dry-run
./import-reviews.sh --source upstream
./import-reviews.sh --source offline-candidate
```

Use `--force` only when the destination is non-empty; the script copies the
current destination to a timestamped backup before importing. It never
modifies the source.

Before rollback, back up the entire `tuicr-fork` data directory. Upstream
0.19.1 ignores fork-only durable thread fields and can discard them if it
rewrites copied sessions, so keep fork data separate and treat any explicit
copy back to upstream as lossy.
