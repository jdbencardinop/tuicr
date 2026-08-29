#!/usr/bin/env bash
set -euo pipefail

TARGET=""
BINARY=""
OUTPUT_DIR=""
SOURCE_SHA="${TUICR_BUILD_SHA:-}"

usage() {
  cat <<'USAGE'
Usage: scripts/package-release-artifact.sh --target TRIPLE --binary PATH --output-dir DIR [--source-sha SHA]

Packages one native fork binary with release documentation, provenance, and
a SHA-256 checksum. The binary must run on the current host.
USAGE
}

die() { printf '[package-release-artifact] ERROR: %s\n' "$*" >&2; exit 1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) TARGET="${2:?--target requires a value}"; shift 2 ;;
    --binary) BINARY="${2:?--binary requires a value}"; shift 2 ;;
    --output-dir) OUTPUT_DIR="${2:?--output-dir requires a value}"; shift 2 ;;
    --source-sha) SOURCE_SHA="${2:?--source-sha requires a value}"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[[ -n "$TARGET" && -n "$BINARY" && -n "$OUTPUT_DIR" ]] || { usage; exit 2; }
[[ -x "$BINARY" ]] || die "binary is not executable: $BINARY"
[[ -n "$SOURCE_SHA" ]] || die "source SHA is required"

case "$TARGET" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) os="linux" ;;
  x86_64-apple-darwin|aarch64-apple-darwin) os="macos" ;;
  *) die "unsupported release target: $TARGET" ;;
esac

version="$("$BINARY" --version)"
package_version="${version#tuicr }"
package_version="${package_version%%+*}"
[[ "$version" == "tuicr $package_version+$SOURCE_SHA" ]] ||
  die "binary identity '$version' does not match source SHA '$SOURCE_SHA'"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

scan_secrets() {
  local root="$1"
  local patterns=(
    'ghp_[0-9A-Za-z]{36,}'
    'gh[oesur]_[0-9A-Za-z]{36,}'
    'github_pat_[0-9A-Za-z_]{22,}'
    'glpat[-_][0-9A-Za-z_-]{20,}'
    'AKIA[0-9A-Z]{16}'
    'xox[baprs]-[0-9A-Za-z-]{10,}'
    '-----BEGIN[A-Z ]*PRIVATE KEY-----'
  )
  local allowlist=(
    'ghp_SENTINEL0123456789abcdefABCDEF01234567'
    'glpat-SENTINEL0123456789abcdefABCDEF'
    'glpat-SENTINEL0123456789abcdef'
    'glpat-XyZ_0123456789abcdef'
    'glpat-ABCDEFGHIJKLMNOPQRST'
  )
  local allowed_hash='861868d6e0246f776b867c651da9519f5d398cee945fe26ae2f0e890dcf64032'
  local file pattern value value_hash allowed

  while IFS= read -r -d '' file; do
    for pattern in "${patterns[@]}"; do
      while IFS= read -r value; do
        [[ -n "$value" ]] || continue
        allowed=0
        for exact in "${allowlist[@]}"; do
          [[ "$value" == "$exact" ]] && allowed=1
        done
        value_hash="$(printf '%s' "$value" | sha256_stream)"
        [[ "$value_hash" == "$allowed_hash" ]] && allowed=1
        [[ "$allowed" -eq 1 ]] ||
          die "non-allowlisted secret-shaped value in $(basename "$file")"
      done < <(grep -aoE -- "$pattern" "$file" 2>/dev/null || true)
    done
  done < <(find "$root" -type f -print0)
}

sha256_stream() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{print $1}'
  else
    shasum -a 256 | awk '{print $1}'
  fi
}

mkdir -p "$OUTPUT_DIR"
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/tuicr-release.XXXXXX")"
trap 'rm -rf "$work_dir"' EXIT

base_name="tuicr-${package_version}-${TARGET}"
stage_dir="$work_dir/$base_name"
mkdir -p "$stage_dir"
cp "$BINARY" "$stage_dir/tuicr"
cp LICENSE "$stage_dir/LICENSE"
cp docs/release/INSTALL.md "$stage_dir/INSTALL.md"
cp docs/release/MIGRATION.md "$stage_dir/MIGRATION.md"
cp docs/offline-candidate/PROVIDER-CAPABILITIES.md "$stage_dir/PROVIDER-CAPABILITIES.md"
cp scripts/import-reviews.sh "$stage_dir/import-reviews.sh"
chmod +x "$stage_dir/tuicr" "$stage_dir/import-reviews.sh"

signing="unsigned"
if [[ "$os" == "macos" ]]; then
  codesign --force --sign - "$stage_dir/tuicr"
  codesign --verify --verbose "$stage_dir/tuicr"
  signing="ad-hoc signed; not Developer ID signed or notarized"
fi

cat > "$stage_dir/PROVENANCE.txt" <<EOF
repository=https://github.com/jdbencardinop/tuicr
source_sha=$SOURCE_SHA
version=$package_version
target=$TARGET
rustc=$(rustc --version)
cargo=$(cargo --version)
runner_os=$(uname -s)
runner_arch=$(uname -m)
signing=$signing
EOF

"$stage_dir/tuicr" --help >/dev/null
scan_secrets "$stage_dir"
archive="$OUTPUT_DIR/$base_name.tar.gz"
tar -C "$work_dir" -czf "$archive" "$base_name"
digest="$(sha256 "$archive")"
printf '%s  %s\n' "$digest" "$(basename "$archive")" > "$archive.sha256"
printf '%s\n' "$version" > "$OUTPUT_DIR/$base_name.version.txt"
