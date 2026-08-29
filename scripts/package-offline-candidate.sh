#!/usr/bin/env bash
# Package an OFFLINE-VALIDATED fork-candidate build of tuicr for local macOS
# x86_64 and Linux x86_64. This script does NOT tag, push, or publish
# anything. See docs/offline-candidate/README.md for what "offline
# candidate" means and what is/isn't validated.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# ---------------------------------------------------------------------------
# Defaults / configuration
# ---------------------------------------------------------------------------
OUTPUT_DIR=""
SKIP_MACOS=0
SKIP_LINUX=0
KEEP_WORK_DIR=0
LINUX_BUILD_IMAGE="rust:1.97-bookworm"
LINUX_RUNTIME_IMAGE="debian:bookworm-slim"
LINUX_PLATFORM="linux/amd64"
# One documented environmental test failure that is not caused by this
# script or by fork changes: `libgit2` cannot discover a worktree when the
# *test process itself* isn't running inside a real git checkout in some
# sandboxes. See prior audits (`offline-integration` todo history). Any
# other failure aborts packaging.
KNOWN_TEST_FAILURES=(
  "vcs::git::libgit2::tests::should_discover_worktree_with_relativeworktrees_extension"
)

usage() {
  cat <<'USAGE' >&2
Usage: scripts/package-offline-candidate.sh --output-dir DIR [options]

Required:
  --output-dir DIR        Directory to write archives/checksums/manifest to.
                           Created if missing. Must not be inside the
                           tracked source tree.

Options:
  --skip-macos             Skip the native macOS x86_64 build/package.
  --skip-linux              Skip the Linux x86_64 container build/package.
  --linux-build-image IMG   Pinned Rust image to build Linux in.
                            Default: rust:1.97-bookworm
  --linux-runtime-image IMG Image used only to run/verify the built Linux
                            binary (no Rust toolchain needed there).
                            Default: debian:bookworm-slim
  --keep-work-dir           Do not delete the scratch working directory
                            (useful for debugging a failed run).
  -h, --help                Show this help.

This script:
  - refuses to run against a dirty tracked-source tree;
  - always builds with `cargo build --release --locked`;
  - never packages `.tpatch` files, credentials, session data, or any
    sibling wayfinder/candidate research data;
  - is OFFLINE-VALIDATED ONLY: it does not tag, push, or publish anything.
USAGE
}

log() { printf '[package-offline-candidate] %s\n' "$*" >&2; }
die() { printf '[package-offline-candidate] ERROR: %s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir)
      OUTPUT_DIR="${2:?--output-dir requires a value}"
      shift 2
      ;;
    --skip-macos) SKIP_MACOS=1; shift ;;
    --skip-linux) SKIP_LINUX=1; shift ;;
    --linux-build-image)
      LINUX_BUILD_IMAGE="${2:?--linux-build-image requires a value}"
      shift 2
      ;;
    --linux-runtime-image)
      LINUX_RUNTIME_IMAGE="${2:?--linux-runtime-image requires a value}"
      shift 2
      ;;
    --keep-work-dir) KEEP_WORK_DIR=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (see --help)" ;;
  esac
done

[[ -n "$OUTPUT_DIR" ]] || { usage; die "--output-dir is required"; }

# ---------------------------------------------------------------------------
# Safety gate: refuse a dirty tracked-source tree
# ---------------------------------------------------------------------------
git -C "$ROOT_DIR" rev-parse --is-inside-work-tree >/dev/null 2>&1 \
  || die "not a Git repository: $ROOT_DIR"

DIRTY_STATUS="$(git -C "$ROOT_DIR" status --porcelain --untracked-files=no)"
if [[ -n "$DIRTY_STATUS" ]]; then
  die "tracked source has uncommitted changes; commit or stash before packaging:
$DIRTY_STATUS"
fi

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR="$(cd "$OUTPUT_DIR" && pwd)"

case "$OUTPUT_DIR" in
  "$ROOT_DIR"/*|"$ROOT_DIR")
    die "--output-dir must not be inside the tracked source tree ($ROOT_DIR); built artifacts are never committed"
    ;;
esac

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/tuicr-offline-candidate.XXXXXX")"
cleanup() {
  if [[ "$KEEP_WORK_DIR" -eq 0 ]]; then
    rm -rf "$WORK_DIR"
  else
    log "keeping work dir: $WORK_DIR"
  fi
}
trap cleanup EXIT

log "work dir: $WORK_DIR"
log "output dir: $OUTPUT_DIR"

# ---------------------------------------------------------------------------
# Source identity
# ---------------------------------------------------------------------------
SOURCE_SHA_FULL="$(git -C "$ROOT_DIR" rev-parse HEAD)"
SOURCE_SHA_SHORT="$(git -C "$ROOT_DIR" rev-parse --short HEAD)"
SOURCE_BRANCH="$(git -C "$ROOT_DIR" rev-parse --abbrev-ref HEAD)"
REPO_REMOTE="$(git -C "$ROOT_DIR" remote get-url origin 2>/dev/null || echo "none")"
# FORK_VERSION is Cargo.toml's full version, e.g. "0.19.1-offline-candidate.1"
# -- a semver prerelease tag that can never collide with an unmodified
# upstream release. UPSTREAM_BASE_VERSION strips that tag back to the
# upstream baseline ("0.19.1") purely for archive-naming/doc-display
# continuity with prior runs of this script.
FORK_VERSION="$(cargo metadata --locked --no-deps --format-version 1 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')"
UPSTREAM_BASE_VERSION="${FORK_VERSION%%-*}"
VERSION="$UPSTREAM_BASE_VERSION"
# Exported so build.rs (both the native macOS build and the Linux container
# build, which builds from a clean `git archive` checkout with no `.git`
# directory to fall back on) embeds an identical, known-correct source SHA
# into `--version` regardless of build environment.
export TUICR_BUILD_SHA="$SOURCE_SHA_SHORT"
EXPECTED_REPORTED_VERSION="tuicr ${FORK_VERSION}+${SOURCE_SHA_SHORT}"
PACKAGED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUSTC_VERSION="$(rustc --version)"
CARGO_VERSION="$(cargo --version)"
HOST_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"
LINUX_BUILD_IMAGE_DIGEST="not-built"

log "source commit: $SOURCE_SHA_FULL ($SOURCE_SHA_SHORT) on $SOURCE_BRANCH"
log "upstream baseline version: $UPSTREAM_BASE_VERSION / fork version: $FORK_VERSION"
log "expected --version output: $EXPECTED_REPORTED_VERSION"
log "toolchain: $RUSTC_VERSION / $CARGO_VERSION (host: $HOST_TRIPLE)"

ARTIFACTS_JSONL="$WORK_DIR/artifacts.jsonl"
: > "$ARTIFACTS_JSONL"

# ---------------------------------------------------------------------------
# Pre-build checks (run once; shared by both platform builds)
# ---------------------------------------------------------------------------
FMT_RESULT="not-run"
CLIPPY_RESULT="not-run"
TEST_RESULT="not-run"
TEST_PASSED=0
TEST_FAILED_COUNT=0
TEST_IGNORED=0

run_prebuild_checks() {
  log "cargo fmt --all --check"
  if cargo fmt --all --check >"$WORK_DIR/fmt.log" 2>&1; then
    FMT_RESULT="passed"
  else
    cat "$WORK_DIR/fmt.log" >&2
    die "cargo fmt --check failed; see above"
  fi

  log "cargo clippy --locked --all-targets -- -D warnings"
  if cargo clippy --locked --all-targets -- -D warnings >"$WORK_DIR/clippy.log" 2>&1; then
    CLIPPY_RESULT="passed"
  else
    cat "$WORK_DIR/clippy.log" >&2
    die "cargo clippy failed; see above"
  fi

  log "cargo test --locked --lib"
  set +e
  cargo test --locked --lib >"$WORK_DIR/test.log" 2>&1
  local test_exit=$?
  set -e
  tail -n 40 "$WORK_DIR/test.log" >&2

  TEST_PASSED="$(grep -Eo '[0-9]+ passed' "$WORK_DIR/test.log" | tail -1 | awk '{print $1}')"
  TEST_FAILED_COUNT="$(grep -Eo '[0-9]+ failed' "$WORK_DIR/test.log" | tail -1 | awk '{print $1}')"
  TEST_IGNORED="$(grep -Eo '[0-9]+ ignored' "$WORK_DIR/test.log" | tail -1 | awk '{print $1}')"
  TEST_PASSED="${TEST_PASSED:-0}"
  TEST_FAILED_COUNT="${TEST_FAILED_COUNT:-0}"
  TEST_IGNORED="${TEST_IGNORED:-0}"

  local failed_tests=()
  while IFS= read -r line; do
    [[ -n "$line" ]] && failed_tests+=("$line")
  done < <(grep -E '^test .* FAILED$' "$WORK_DIR/test.log" | awk '{print $2}' || true)

  local unexpected=()
  for failed_test in "${failed_tests[@]:-}"; do
    [[ -z "$failed_test" ]] && continue
    local known=0
    for allowed in "${KNOWN_TEST_FAILURES[@]}"; do
      [[ "$failed_test" == "$allowed" ]] && known=1 && break
    done
    [[ "$known" -eq 0 ]] && unexpected+=("$failed_test")
  done

  if [[ "${#unexpected[@]}" -gt 0 ]]; then
    die "unexpected test failures, aborting packaging: ${unexpected[*]}"
  fi
  if [[ "$test_exit" -ne 0 && "${#failed_tests[@]}" -eq 0 ]]; then
    die "cargo test exited non-zero with no parsed FAILED lines; see $WORK_DIR/test.log"
  fi

  TEST_RESULT="passed (${TEST_FAILED_COUNT} known pre-existing environmental failure(s) allowlisted)"
  log "tests: $TEST_PASSED passed, $TEST_FAILED_COUNT failed (allowlisted), $TEST_IGNORED ignored"
}

# ---------------------------------------------------------------------------
# cargo metadata + a from-existing-data license summary (no new tool added)
# ---------------------------------------------------------------------------
generate_dependency_snapshot() {
  local manifest_dir="$1"
  mkdir -p "$manifest_dir"
  cargo metadata --locked --format-version 1 > "$manifest_dir/cargo-metadata.json"

  python3 - "$manifest_dir/cargo-metadata.json" "$manifest_dir/THIRD-PARTY-LICENSES.md" <<'PYEOF'
import json, sys
from collections import defaultdict

meta_path, out_path = sys.argv[1], sys.argv[2]
with open(meta_path) as f:
    meta = json.load(f)

used_ids = {n["id"] for n in meta.get("resolve", {}).get("nodes", [])}

by_license = defaultdict(set)
missing = set()
for pkg in meta["packages"]:
    if pkg["id"] not in used_ids:
        continue
    lic = pkg.get("license") or None
    entry = f"{pkg['name']} {pkg['version']}"
    if lic:
        by_license[lic].add(entry)
    else:
        missing.add(entry)

with open(out_path, "w") as out:
    out.write("# Third-party dependency license summary\n\n")
    out.write(
        "Generated from `cargo metadata --locked` (each crate's own declared "
        "`license` field, as published to the registry) -- no separate "
        "license-audit tool was added for this. **This is an audit aid, not "
        "legal certification.** It is a summary for human review only; "
        "verify anything load-bearing against the vendored source pinned in "
        "Cargo.lock before redistribution decisions. It also does not cover "
        "this repository's own MIT license/attribution -- see the bundled "
        "`LICENSE` file (upstream MIT, unmodified) in each packaged "
        "archive.\n\n"
    )
    out.write(f"Total third-party packages in the resolved dependency graph: {len(used_ids) - 1}\n\n")
    out.write("## By declared license\n\n")
    for lic in sorted(by_license):
        pkgs = sorted(by_license[lic])
        out.write(f"### {lic} ({len(pkgs)} package(s))\n\n")
        for p in pkgs:
            out.write(f"- {p}\n")
        out.write("\n")
    if missing:
        out.write("## Packages with no declared `license` field (needs manual review)\n\n")
        for p in sorted(missing):
            out.write(f"- {p}\n")
print(f"license summary written: {out_path}", file=sys.stderr)
PYEOF
}

# ---------------------------------------------------------------------------
# Secret scan of the exact tracked source that will be built and of the
# staged archive contents (binary + docs) before packaging. No new tool is
# added -- this is a small grep-based pattern scan, recorded as an artifact
# for human review, not a certification that no secret exists anywhere.
#
# Byte-aware and explicit-file-list by design: every regular file under the
# scan root -- including the compiled binary and every file that will end up
# inside the tar archive -- is scanned with `grep -a` (never plain `grep -I`,
# which silently treats files it guesses are "binary" as non-matching and
# would make a compiled binary invisible to the scan while a report still
# claims it was covered). Matched secret *values* are never printed or
# written to the report; only the relative filename and the matched pattern
# category are recorded. Any packaging run with a non-allowlisted match is
# aborted (`die`) before an archive is ever created -- see package_artifact().
# ---------------------------------------------------------------------------
SECRET_SCAN_PATTERN_IDS=(
  github-pat-classic
  github-pat-other-prefixes
  github-pat-fine-grained
  gitlab-pat
  aws-access-key-id
  slack-token
  pem-private-key
)
SECRET_SCAN_PATTERNS=(
  'ghp_[0-9A-Za-z]{36,}'
  'gh[oesur]_[0-9A-Za-z]{36,}'
  'github_pat_[0-9A-Za-z_]{22,}'
  'glpat[-_][0-9A-Za-z_-]{20,}'
  'AKIA[0-9A-Z]{16}'
  'xox[baprs]-[0-9A-Za-z-]{10,}'
  '-----BEGIN[A-Z ]*PRIVATE KEY-----'
)

# Single documented source of truth for which credential-shaped *prefixes*
# the patterns above must cover: this list is a manually-kept mirror of
# `KNOWN_TOKEN_PREFIXES` in `src/forge/mod.rs` (the app's own defense-in-depth
# redaction prefix list). It exists so the two lists can be verified
# byte-for-byte identical, and so every prefix can be proven actually
# detected, every time this script runs -- see
# `_secret_scan_check_prefix_sync()` in `self_test_secret_scan()` below. If
# `src/forge/mod.rs` ever adds, removes, or renames a prefix without this
# array (and the patterns above) being updated to match, packaging fails
# loudly instead of silently losing scan coverage. See
# docs/offline-candidate/SECRET-SCAN.md's "Prefix synchronization contract".
SECRET_SCAN_KNOWN_TOKEN_PREFIXES=(
  'ghp_'
  'gho_'
  'ghu_'
  'ghs_'
  'ghr_'
  'github_pat_'
  'glpat-'
  'glpat_'
)

# Narrow, exact-value allowlist. Every entry here is a complete, literal
# matched string that must appear byte-for-byte in `SECRET_SCAN_ALLOWLIST`
# to be accepted -- never a path, directory, filename, or value *prefix*.
# These are all deliberately synthetic test-fixture tokens already present,
# unmodified, in this repo's own tracked test source (see SECRET-SCAN.md for
# the exact file:line provenance of each entry); none of them are real
# credentials. Any other match -- including a longer/shorter variant of one
# of these strings -- is treated as a real, non-allowlisted hit and fails
# packaging.
SECRET_SCAN_ALLOWLIST=(
  'ghp_SENTINEL0123456789abcdefABCDEF01234567'
  'glpat-SENTINEL0123456789abcdefABCDEF'
  'glpat-SENTINEL0123456789abcdef'
  'glpat-XyZ_0123456789abcdef'
  'glpat-ABCDEFGHIJKLMNOPQRST'
)

# Second, deliberately much narrower allowlist mechanism: SHA-256 digests
# (not plaintext) of specific, individually-investigated matched values that
# are compiled INTO a dependency's own binary output (not this project's
# source), where recording the literal matched text in this tracked script
# would itself mean permanently committing a secret-shaped string. Every
# entry here must be documented in docs/offline-candidate/SECRET-SCAN.md with
# the forensic evidence that ruled out a real credential (byte length,
# character-class/entropy analysis, build context, non-reproduction in an
# isolated minimal build, absence from all searched dependency source).
# This is NOT a general escape hatch -- it exists only for this narrow
# "confirmed-non-secret compiled-artifact noise, but unsafe to quote
# verbatim" case, and every entry must have a documented investigation.
SECRET_SCAN_ALLOWLIST_SHA256=(
  '861868d6e0246f776b867c651da9519f5d398cee945fe26ae2f0e890dcf64032'
)

_secret_scan_is_allowlisted() {
  local value="$1" entry value_sha256
  for entry in "${SECRET_SCAN_ALLOWLIST[@]}"; do
    [[ "$value" == "$entry" ]] && return 0
  done
  # Only hashed if the plaintext allowlist missed, since shasum spawns a
  # subprocess per call.
  value_sha256="$(printf '%s' "$value" | shasum -a 256 | awk '{print $1}')"
  for entry in "${SECRET_SCAN_ALLOWLIST_SHA256[@]}"; do
    [[ "$value_sha256" == "$entry" ]] && return 0
  done
  return 1
}

# Scans every regular file under $1, byte-aware (`grep -a`, no binary skip),
# against every pattern in SECRET_SCAN_PATTERNS. Emits one
# "relative/path<TAB>pattern-id" line on stdout per NON-allowlisted match
# found (never the matched value itself). Emits nothing and exits 0 if no
# non-allowlisted match is found. Never calls `die` itself -- callers decide
# whether/how to fail so this can also be used, side-effect-free, by the
# self-test below.
_secret_scan_detect() {
  local scan_root="$1"
  local file rel pattern_idx pattern pattern_id value
  while IFS= read -r -d '' file; do
    rel="${file#"$scan_root"/}"
    for pattern_idx in "${!SECRET_SCAN_PATTERNS[@]}"; do
      pattern="${SECRET_SCAN_PATTERNS[$pattern_idx]}"
      pattern_id="${SECRET_SCAN_PATTERN_IDS[$pattern_idx]}"
      while IFS= read -r value; do
        [[ -z "$value" ]] && continue
        if ! _secret_scan_is_allowlisted "$value"; then
          printf '%s\t%s\n' "$rel" "$pattern_id"
        fi
      done < <(grep -aoE "$pattern" "$file" 2>/dev/null || true)
    done
  done < <(find "$scan_root" -type f ! -path '*/.git/*' -print0)
}

# Extracts the `KNOWN_TOKEN_PREFIXES` string literals from
# src/forge/mod.rs (the app's own defense-in-depth redaction prefix list),
# in the exact form used in that Rust source. Used by
# `self_test_secret_scan` to prove `SECRET_SCAN_KNOWN_TOKEN_PREFIXES` above
# has not silently drifted out of sync with it.
_secret_scan_extract_source_prefixes() {
  python3 - "$ROOT_DIR/src/forge/mod.rs" <<'PYEOF'
import re
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    text = f.read()

match = re.search(
    r"const KNOWN_TOKEN_PREFIXES:\s*&\[&str\]\s*=\s*&\[(.*?)\];",
    text,
    re.DOTALL,
)
if not match:
    print("ERROR: could not find KNOWN_TOKEN_PREFIXES in src/forge/mod.rs", file=sys.stderr)
    sys.exit(1)

for literal in re.findall(r'"([^"]*)"', match.group(1)):
    print(literal)
PYEOF
}

# Proves the scanner is byte-aware (does not silently skip binary files, the
# bug this hardening round fixes) and that the allowlist is genuinely narrow
# (a brand-new synthetic secret-shaped value is NOT waved through), before
# any real scan result is trusted. Aborts packaging if either check fails.
#
# Also proves prefix coverage cannot silently drift: `_secret_scan_check_
# prefix_sync` below asserts `SECRET_SCAN_KNOWN_TOKEN_PREFIXES` is byte-for-
# byte identical (as a set) to `KNOWN_TOKEN_PREFIXES` in src/forge/mod.rs,
# then proves every single one of those prefixes is actually detected by
# SECRET_SCAN_PATTERNS via a synthetic non-allowlisted payload -- not just
# documented as covered.
self_test_secret_scan() {
  local test_root="$WORK_DIR/self-test-secret-scan"
  rm -rf "$test_root"
  mkdir -p "$test_root/clean-case" "$test_root/dirty-case"

  # Non-text "binary" payload -- plain `grep -I`/`grep` (no -a) would guess
  # this file is binary and skip it entirely, which is exactly the bug being
  # fixed. Both the clean and dirty dummy files share this same binary
  # prefix; only the dirty one has a secret-shaped suffix appended.
  printf '\x00\x01\x02\x03\xff\xfe\xfd\x00binary-blob-marker\x00\x01\x02' > "$test_root/clean-case/dummy.bin"
  cp "$test_root/clean-case/dummy.bin" "$test_root/dirty-case/dummy.bin"
  # Deliberately synthetic, NOT in SECRET_SCAN_ALLOWLIST, NOT a real
  # credential -- proves both binary-awareness and allowlist narrowness.
  # Split into two fragments (only concatenated at runtime, into a
  # never-committed scratch file) so this fixture never appears as a
  # matching contiguous literal in this script's own tracked source --
  # otherwise the real "clean source checkout" scan would flag this file.
  local self_test_token_prefix="ghp_"
  local self_test_token_body="SELFTESTSYNTHETICNOTREALTOKEN1234567"
  printf '%s%s' "$self_test_token_prefix" "$self_test_token_body" >> "$test_root/dirty-case/dummy.bin"

  local clean_hits dirty_hits
  clean_hits="$(_secret_scan_detect "$test_root/clean-case")"
  dirty_hits="$(_secret_scan_detect "$test_root/dirty-case")"

  [[ -z "$clean_hits" ]] \
    || die "secret-scan self-test failed: scanner reported a false positive on a clean dummy binary file with no secret-shaped content; refusing to trust it for the real scan"
  [[ -n "$dirty_hits" ]] \
    || die "secret-scan self-test failed: scanner did NOT detect a synthetic non-allowlisted secret-shaped token appended to a dummy binary file (binary-skip / false-negative bug); refusing to trust it for the real scan"

  rm -rf "$test_root"
  log "secret-scan self-test passed: dummy binary with a synthetic non-allowlisted token is flagged, clean dummy binary is not"

  # ---- Prefix synchronization + per-prefix detection coverage check -------
  local source_prefixes mirror_sorted source_sorted
  source_prefixes="$(_secret_scan_extract_source_prefixes)"
  mirror_sorted="$(printf '%s\n' "${SECRET_SCAN_KNOWN_TOKEN_PREFIXES[@]}" | sort)"
  source_sorted="$(printf '%s\n' "$source_prefixes" | sort)"
  [[ "$mirror_sorted" == "$source_sorted" ]] \
    || die "secret-scan self-test failed: SECRET_SCAN_KNOWN_TOKEN_PREFIXES in this script no longer matches KNOWN_TOKEN_PREFIXES in src/forge/mod.rs (prefix list drift). Update both SECRET_SCAN_KNOWN_TOKEN_PREFIXES and SECRET_SCAN_PATTERNS above to cover every current prefix, then update docs/offline-candidate/SECRET-SCAN.md."

  local prefix_test_root="$WORK_DIR/self-test-secret-scan-prefixes"
  rm -rf "$prefix_test_root"
  mkdir -p "$prefix_test_root"
  # Same fragment-split technique as above: the filler body is a plain,
  # non-secret-shaped array element on its own, and is only concatenated
  # with a real credential prefix at runtime into a scratch file that is
  # never committed -- so no prefix+full-token literal ever appears
  # contiguously in this script's own tracked source.
  local prefix_test_body="SELFTESTPREFIXCOVERAGENOTREALTOKEN1234567890"
  local prefix idx=0 file
  for prefix in "${SECRET_SCAN_KNOWN_TOKEN_PREFIXES[@]}"; do
    idx=$((idx + 1))
    file="$prefix_test_root/prefix-$idx.bin"
    printf '\x00\x01\x02\x03binary-blob-marker\x00\x01\x02' > "$file"
    printf '%s%s' "$prefix" "$prefix_test_body" >> "$file"
    if [[ -z "$(_secret_scan_detect "$prefix_test_root")" ]]; then
      die "secret-scan self-test failed: a synthetic non-allowlisted token using known credential prefix '$prefix' (from KNOWN_TOKEN_PREFIXES in src/forge/mod.rs) was NOT detected by any SECRET_SCAN_PATTERNS regex; the scanner has a real prefix-coverage gap. Update SECRET_SCAN_PATTERNS above so every prefix in SECRET_SCAN_KNOWN_TOKEN_PREFIXES is actually matched."
    fi
    rm -f "$file"
  done
  rm -rf "$prefix_test_root"
  log "secret-scan prefix-coverage self-test passed: SECRET_SCAN_KNOWN_TOKEN_PREFIXES matches src/forge/mod.rs's KNOWN_TOKEN_PREFIXES (${#SECRET_SCAN_KNOWN_TOKEN_PREFIXES[@]} prefixes), and each one is independently detected"
}

run_secret_scan() {
  local scan_root="$1" label="$2" report_file="$3"
  local hits_raw hit_count=0 rel pattern_id
  hits_raw="$(_secret_scan_detect "$scan_root")"
  {
    echo "# Secret scan: $label"
    echo
    echo "Byte-aware pattern scan (\`grep -a\`, explicit per-file enumeration,"
    echo "no binary-file skip) over every regular file under \`$scan_root\`"
    echo "(tracked source / staged archive contents -- including the"
    echo "compiled binary -- at commit $SOURCE_SHA_SHORT). This is an audit"
    echo "aid, not a certification that no secret exists anywhere. Matched"
    echo "secret *values* are never recorded here, only filename + pattern"
    echo "category. Values exactly matching the narrow, documented allowlists"
    echo "in scripts/package-offline-candidate.sh's SECRET_SCAN_ALLOWLIST"
    echo "(literal synthetic test fixtures) or SECRET_SCAN_ALLOWLIST_SHA256"
    echo "(SHA-256 digests of specific investigated compiled-dependency"
    echo "false positives) are known, documented non-secrets and do not fail"
    echo "packaging (see docs/offline-candidate/SECRET-SCAN.md for provenance"
    echo "and design of both); every other match aborts packaging immediately."
    echo
  } >> "$report_file"
  if [[ -n "$hits_raw" ]]; then
    while IFS=$'\t' read -r rel pattern_id; do
      [[ -z "$rel" ]] && continue
      hit_count=$((hit_count + 1))
      echo "- NON-ALLOWLISTED MATCH: \`$rel\` (pattern category: $pattern_id)" >> "$report_file"
    done <<< "$hits_raw"
  fi
  if [[ "$hit_count" -eq 0 ]]; then
    echo "No non-allowlisted matches for any of the ${#SECRET_SCAN_PATTERNS[@]} known secret patterns." >> "$report_file"
    log "[$label] secret scan: no non-allowlisted matches"
  else
    log "[$label] secret scan: $hit_count non-allowlisted match(es) -- see $report_file"
    die "[$label] secret scan found $hit_count non-allowlisted potential secret(s); refusing to package (filenames/categories only are in $report_file, no values)"
  fi
}

# ---------------------------------------------------------------------------
# Fixture generation for smoke tests (self-contained; a small deterministic
# fixture, not the full 1,000-line wayfinder evaluation fixture -- just
# enough to exercise real CLI code paths end to end).
# ---------------------------------------------------------------------------
make_smoke_fixture() {
  local fixture_dir="$1"
  mkdir -p "$fixture_dir"
  git -C "$fixture_dir" init -q -b main
  git -C "$fixture_dir" config user.email "smoke@example.invalid"
  git -C "$fixture_dir" config user.name "offline-candidate-smoke"
  printf 'line1\nline2\nline3\n' > "$fixture_dir/a.txt"
  git -C "$fixture_dir" add a.txt
  git -C "$fixture_dir" commit -q -m init
  printf 'line1\nCHANGED\nline3\nline4\n' > "$fixture_dir/a.txt"
  git -C "$fixture_dir" add -A
  git -C "$fixture_dir" commit -q -m change
}

# Generate a minimal, schema-valid ReviewSession JSON via the crate's own
# public API (rather than hand-rolling private serde internals). The example
# source is written to examples/ and deleted within this function; it is
# never committed. JSON text has no target-arch dependency, so this is
# always built for the host, even when producing a session used to smoke
# the Linux artifact.
generate_seed_session() {
  local repo_path="$1" base="$2" head="$3" out_file="$4"
  local example_name="_offline_candidate_smoke_session"
  local example_path="$ROOT_DIR/examples/${example_name}.rs"

  cat > "$example_path" <<'RSEOF'
//! Throwaway helper used only by scripts/package-offline-candidate.sh's
//! offline fixture smoke test. Written immediately before use and deleted
//! immediately after by that script; never committed.
use std::path::PathBuf;

use tuicr::model::review::{ReviewSession, SessionDiffSource};

fn main() {
    let mut args = std::env::args().skip(1);
    let repo_path = PathBuf::from(args.next().expect("usage: <repo_path> <base> <head>"));
    let base = args.next().expect("usage: <repo_path> <base> <head>");
    let head = args.next().expect("usage: <repo_path> <base> <head>");

    let mut session = ReviewSession::new(
        repo_path,
        base.clone(),
        Some(head.clone()),
        SessionDiffSource::CommitRange,
    );
    session.commit_range = Some(vec![base, head]);

    print!(
        "{}",
        serde_json::to_string_pretty(&session).expect("session serializes")
    );
}
RSEOF

  if ! cargo build --locked --release --example "$example_name" >"$WORK_DIR/seed-session-build.log" 2>&1; then
    cat "$WORK_DIR/seed-session-build.log" >&2
    rm -f "$example_path"
    die "failed to build seed-session helper"
  fi
  "$ROOT_DIR/target/release/examples/${example_name}" "$repo_path" "$base" "$head" > "$out_file"
  rm -f "$example_path" "$ROOT_DIR/target/release/examples/${example_name}"
}

# ---------------------------------------------------------------------------
# Offline review/thread/dry-run CLI smoke. Fully isolated from real user
# data (its own HOME/XDG dirs, created fresh under $WORK_DIR) and makes no
# external network calls. Runs either natively (macOS) or inside the pinned
# Linux runtime container, dispatched by $SMOKE_MODE.
#
# $1: path to the extracted binary to test
# $2: short label for logging (e.g. "macos-x86_64")
# ---------------------------------------------------------------------------
SMOKE_MODE="native"   # native | docker

tuicr_smoke_exec() {
  local iso_home="$1" bin_path="$2"
  shift 2
  case "$SMOKE_MODE" in
    native)
      env HOME="$iso_home" XDG_CONFIG_HOME="$iso_home/.config" XDG_DATA_HOME="$iso_home/.local/share" \
        "$bin_path" "$@"
      ;;
    docker)
      docker run --rm --platform "$LINUX_PLATFORM" \
        -v "$WORK_DIR:$WORK_DIR" \
        -e HOME="$iso_home" -e XDG_CONFIG_HOME="$iso_home/.config" -e XDG_DATA_HOME="$iso_home/.local/share" \
        -w "$WORK_DIR" \
        "$LINUX_RUNTIME_IMAGE" \
        "$bin_path" "$@"
      ;;
    *) die "unknown SMOKE_MODE: $SMOKE_MODE" ;;
  esac
}

run_review_cli_smoke() {
  local bin_path="$1" label="$2"
  local smoke_root="$WORK_DIR/smoke-$label"
  local fixture_dir="$smoke_root/fixture-repo"
  local iso_home="$smoke_root/home"
  rm -rf "$smoke_root"
  mkdir -p "$iso_home"
  make_smoke_fixture "$fixture_dir"

  local base head
  base="$(git -C "$fixture_dir" rev-parse HEAD~1)"
  head="$(git -C "$fixture_dir" rev-parse HEAD)"

  local seed_session="$smoke_root/seed-session.json"
  generate_seed_session "$fixture_dir" "$base" "$head" "$seed_session"

  log "[$label] review comments (direct path, expect [])"
  local out
  out="$(tuicr_smoke_exec "$iso_home" "$bin_path" review comments --session "$seed_session")"
  [[ "$out" == "[]" ]] || die "[$label] expected empty comments, got: $out"

  log "[$label] review add (legacy comment)"
  tuicr_smoke_exec "$iso_home" "$bin_path" review add --session "$seed_session" --username smoke-bot "smoke test comment" >/dev/null

  log "[$label] review list --repo (resolve canonical slug)"
  local slug
  slug="$(tuicr_smoke_exec "$iso_home" "$bin_path" review list --repo "$fixture_dir" \
    | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d[0]["slug"])')"
  [[ -n "$slug" ]] || die "[$label] no session slug resolved after review add"

  log "[$label] review comments --session <slug> (expect 1)"
  local comment_count
  comment_count="$(tuicr_smoke_exec "$iso_home" "$bin_path" review comments --session "$slug" --repo "$fixture_dir" \
    | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
  [[ "$comment_count" == "1" ]] || die "[$label] expected 1 comment, got $comment_count"

  log "[$label] review thread add"
  tuicr_smoke_exec "$iso_home" "$bin_path" review thread add --session "$slug" --repo "$fixture_dir" --author smoke-bot "smoke thread body" >/dev/null

  log "[$label] review thread list (expect >= 1)"
  local thread_count
  thread_count="$(tuicr_smoke_exec "$iso_home" "$bin_path" review thread list --session "$slug" --repo "$fixture_dir" \
    | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
  [[ "$thread_count" -ge 1 ]] || die "[$label] expected at least 1 thread, got $thread_count"

  log "[$label] review publish --dry-run --provider github (no network; local plan only)"
  local plan_ops
  plan_ops="$(tuicr_smoke_exec "$iso_home" "$bin_path" review publish --session "$slug" --repo "$fixture_dir" --dry-run --provider github \
    | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["operations"]))')"
  [[ "$plan_ops" -ge 1 ]] || die "[$label] expected at least 1 planned operation, got $plan_ops"

  log "[$label] review/thread/dry-run CLI smoke passed ($comment_count comment(s), $thread_count thread(s), $plan_ops planned op(s))"
  rm -rf "$smoke_root"
}

# ---------------------------------------------------------------------------
# Packaging: binary + LICENSE + offline-candidate docs -> tar.gz + checksums
# ---------------------------------------------------------------------------
package_artifact() {
  local os="$1" arch="$2" binary_path="$3" build_command="$4" target_triple="$5"
  local base_name="tuicr-offline-candidate-${VERSION}-${SOURCE_SHA_SHORT}-${os}-${arch}"
  local stage_root="$WORK_DIR/stage-${os}-${arch}"
  local stage_dir="$stage_root/${base_name}"
  rm -rf "$stage_root"
  mkdir -p "$stage_dir"

  cp "$binary_path" "$stage_dir/tuicr"
  chmod +x "$stage_dir/tuicr"
  cp "$ROOT_DIR/LICENSE" "$stage_dir/LICENSE"

  local sed_script
  sed_script="s/@@ARCHIVE_NAME@@/${base_name}/g;"
  sed_script+="s/@@FORK_VERSION@@/${FORK_VERSION}/g;"
  sed_script+="s/@@VERSION@@/${VERSION}/g;"
  sed_script+="s/@@SOURCE_SHA_SHORT@@/${SOURCE_SHA_SHORT}/g;"
  sed_script+="s/@@SOURCE_SHA_FULL@@/${SOURCE_SHA_FULL}/g;"
  sed_script+="s/@@OS@@/${os}/g;"
  sed_script+="s/@@ARCH@@/${arch}/g;"
  sed_script+="s/@@PACKAGED_AT@@/${PACKAGED_AT}/g;"

  for doc in README.md INSTALL.md MIGRATION.md PROVIDER-CAPABILITIES.md; do
    sed -e "$sed_script" "$ROOT_DIR/docs/offline-candidate/$doc" > "$stage_dir/$doc"
  done
  cp "$ROOT_DIR/docs/offline-candidate/config.no-update-check.toml" "$stage_dir/config.no-update-check.toml"
  cp "$ROOT_DIR/scripts/import-upstream-reviews.sh" "$stage_dir/import-upstream-reviews.sh"
  chmod +x "$stage_dir/import-upstream-reviews.sh"

  # Secret scan every file that will go into the archive -- including the
  # compiled binary itself -- BEFORE the archive is created (`tar`, below).
  # This must run first: a non-allowlisted match aborts packaging (die)
  # before any archive exists to checksum or ship, so a real secret can
  # never reach a produced/checksummed artifact.
  run_secret_scan "$stage_dir" "$os-$arch staged archive (pre-tar)" "$OUTPUT_DIR/manifest/SECRET-SCAN.md"

  local archive_path="$OUTPUT_DIR/${base_name}.tar.gz"
  rm -f "$archive_path"
  tar -C "$stage_root" -czf "$archive_path" "$base_name"

  local binary_sha256 archive_sha256 binary_size archive_size
  binary_sha256="$(shasum -a 256 "$stage_dir/tuicr" | awk '{print $1}')"
  archive_sha256="$(shasum -a 256 "$archive_path" | awk '{print $1}')"
  binary_size="$(wc -c < "$stage_dir/tuicr" | tr -d ' ')"
  archive_size="$(wc -c < "$archive_path" | tr -d ' ')"

  # --- Verify: fresh extract + run --version + full CLI smoke ------------
  local verify_dir="$WORK_DIR/verify-${os}-${arch}"
  rm -rf "$verify_dir"
  mkdir -p "$verify_dir"
  tar -C "$verify_dir" -xzf "$archive_path"
  local extracted_bin="$verify_dir/$base_name/tuicr"
  [[ -x "$extracted_bin" ]] || die "extracted binary missing or not executable: $extracted_bin"

  if [[ "$os" == "linux" ]]; then
    SMOKE_MODE="docker"
  else
    SMOKE_MODE="native"
  fi

  local reported_version
  reported_version="$(tuicr_smoke_exec "$WORK_DIR/home-version-check-$os" "$extracted_bin" --version)"
  [[ "$reported_version" == "$EXPECTED_REPORTED_VERSION" ]] \
    || die "[$os] unexpected --version output: got '$reported_version', expected '$EXPECTED_REPORTED_VERSION'"
  tuicr_smoke_exec "$WORK_DIR/home-version-check-$os" "$extracted_bin" --help >/dev/null

  run_review_cli_smoke "$extracted_bin" "$os-$arch"

  cat >> "$ARTIFACTS_JSONL" <<EOF
{"os":"$os","arch":"$arch","filename":"$(basename "$archive_path")","sha256":"$archive_sha256","size_bytes":$archive_size,"binary_sha256":"$binary_sha256","binary_size_bytes":$binary_size,"build_command":"$build_command","build_target_triple":"$target_triple","reported_version":"$reported_version","extract_verified":true,"fixture_smoke_verified":true}
EOF

  rm -rf "$verify_dir"
  log "packaged: $archive_path"
  log "  sha256: $archive_sha256"
  log "  size:   $archive_size bytes"
}

# ---------------------------------------------------------------------------
# Clean-checkout source copy used for BOTH platform builds (not the live
# working tree). This is a `git archive` of the exact packaged commit, so
# only tracked files at that commit are present -- no `.tpatch` scratch
# files, no untracked session/audit data, no `.git` internals, no local
# env/config -- regardless of what else might exist untracked in the
# working tree. The dirty-tree gate above already guarantees the tracked
# tree matches this commit exactly.
# ---------------------------------------------------------------------------
make_clean_source_checkout() {
  local dest_dir="$1"
  mkdir -p "$dest_dir"
  git -C "$ROOT_DIR" archive --format=tar "$SOURCE_SHA_FULL" | tar -x -C "$dest_dir"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
run_prebuild_checks
generate_dependency_snapshot "$OUTPUT_DIR/manifest"

# Prove the secret scanner itself works (byte-aware, narrow allowlist)
# before trusting any of its results below.
self_test_secret_scan

: > "$OUTPUT_DIR/manifest/SECRET-SCAN.md"
{
  echo "Self-test: a synthetic, non-allowlisted secret-shaped token appended"
  echo "to a dummy binary file was correctly flagged, and a clean dummy"
  echo "binary file with no secret-shaped content was correctly not flagged"
  echo "(see docs/offline-candidate/SECRET-SCAN.md for the self-test design)."
  echo "Self-test: PASSED."
  echo
} >> "$OUTPUT_DIR/manifest/SECRET-SCAN.md"

CLEAN_SRC_DIR="$WORK_DIR/clean-src"
log "building a clean git-archive checkout of $SOURCE_SHA_SHORT (used for both platform builds)"
make_clean_source_checkout "$CLEAN_SRC_DIR"
run_secret_scan "$CLEAN_SRC_DIR" "clean source checkout ($SOURCE_SHA_SHORT)" "$OUTPUT_DIR/manifest/SECRET-SCAN.md"

if [[ "$SKIP_MACOS" -eq 0 ]]; then
  if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "x86_64" ]]; then
    log "skipping macOS build: this host is not macOS x86_64 ($(uname -s)/$(uname -m))"
  else
    log "building macOS x86_64 (native, isolated toolchain: $RUSTC_VERSION, clean checkout)"
    (cd "$CLEAN_SRC_DIR" && cargo build --release --locked)
    package_artifact "macos" "x86_64" "$CLEAN_SRC_DIR/target/release/tuicr" \
      "cargo build --release --locked" "x86_64-apple-darwin"
  fi
fi

if [[ "$SKIP_LINUX" -eq 0 ]]; then
  if ! command -v docker >/dev/null 2>&1 || ! docker info >/dev/null 2>&1; then
    log "skipping Linux build: Docker is not available/running"
  else
    log "pulling pinned Linux build image: $LINUX_BUILD_IMAGE ($LINUX_PLATFORM)"
    docker pull --platform "$LINUX_PLATFORM" "$LINUX_BUILD_IMAGE" >&2
    LINUX_BUILD_IMAGE_DIGEST="$(docker image inspect "$LINUX_BUILD_IMAGE" --format '{{index .RepoDigests 0}}' 2>/dev/null || echo "unknown")"
    log "pulling Linux runtime verification image: $LINUX_RUNTIME_IMAGE ($LINUX_PLATFORM)"
    docker pull --platform "$LINUX_PLATFORM" "$LINUX_RUNTIME_IMAGE" >&2

    LINUX_TARGET_DIR="$WORK_DIR/linux-target"
    LINUX_REGISTRY_DIR="$WORK_DIR/linux-registry"
    LINUX_SRC_COPY_DIR="$WORK_DIR/linux-src"
    mkdir -p "$LINUX_TARGET_DIR" "$LINUX_REGISTRY_DIR" "$LINUX_SRC_COPY_DIR"

    log "building Linux x86_64 in pinned container ($LINUX_BUILD_IMAGE @ $LINUX_BUILD_IMAGE_DIGEST)"
    # Copy the SAME clean git-archive checkout used for the macOS build
    # (read-only mount can't hold Cargo's own lock files/incremental
    # caches, so it still needs a container-writable copy) rather than a
    # separate rsync of the live working tree.
    cp -R "$CLEAN_SRC_DIR/." "$LINUX_SRC_COPY_DIR/"
    docker run --rm --platform "$LINUX_PLATFORM" \
      -v "$LINUX_SRC_COPY_DIR:/src" \
      -v "$LINUX_TARGET_DIR:/build-target" \
      -v "$LINUX_REGISTRY_DIR:/usr/local/cargo/registry" \
      -e CARGO_TARGET_DIR=/build-target \
      -e TUICR_BUILD_SHA="$SOURCE_SHA_SHORT" \
      -w /src \
      "$LINUX_BUILD_IMAGE" \
      cargo build --release --locked >&2

    package_artifact "linux" "x86_64" "$LINUX_TARGET_DIR/release/tuicr" \
      "cargo build --release --locked" "x86_64-unknown-linux-gnu"
  fi
fi

# ---------------------------------------------------------------------------
# Checksums + machine-readable manifest
# ---------------------------------------------------------------------------
(
  cd "$OUTPUT_DIR"
  shopt -s nullglob
  archives=(tuicr-offline-candidate-"${VERSION}"-"${SOURCE_SHA_SHORT}"-*.tar.gz)
  if [[ "${#archives[@]}" -gt 0 ]]; then
    shasum -a 256 "${archives[@]}" > CHECKSUMS.sha256
  fi
)

python3 - "$ARTIFACTS_JSONL" "$WORK_DIR/artifacts.json" <<'PYEOF'
import json, sys
artifacts = []
with open(sys.argv[1]) as f:
    for line in f:
        line = line.strip()
        if line:
            artifacts.append(json.loads(line))
with open(sys.argv[2], "w") as out:
    json.dump(artifacts, out)
PYEOF

log "writing manifest.json"
python3 - \
  "$WORK_DIR/artifacts.json" \
  "$OUTPUT_DIR/manifest.json" \
  "$PACKAGED_AT" "$REPO_REMOTE" "$SOURCE_BRANCH" "$SOURCE_SHA_FULL" "$SOURCE_SHA_SHORT" "$VERSION" "$FORK_VERSION" "$EXPECTED_REPORTED_VERSION" \
  "$RUSTC_VERSION" "$CARGO_VERSION" "$LINUX_BUILD_IMAGE" "$LINUX_BUILD_IMAGE_DIGEST" \
  "$FMT_RESULT" "$CLIPPY_RESULT" "$TEST_RESULT" "$TEST_PASSED" "$TEST_FAILED_COUNT" "$TEST_IGNORED" \
<<'PYEOF'
import json, sys

(artifacts_path, out_path, packaged_at, repo_remote, branch, sha_full, sha_short, version, fork_version,
 expected_reported_version, rustc_version, cargo_version, linux_image, linux_image_digest,
 fmt_result, clippy_result, test_result, test_passed, test_failed, test_ignored) = sys.argv[1:]

with open(artifacts_path) as f:
    artifacts = json.load(f)

manifest = {
    "manifest_schema_version": 2,
    "generated_at_utc": packaged_at,
    "release_readiness": "OFFLINE_VALIDATED_ONLY",
    "not_release_ready_because": [
        "no live provider (GitHub/GitLab/Azure DevOps/Gitea/Forgejo) write validation was performed producing this archive",
        "no real Ubuntu-under-WSL validation was performed; the Linux artifact is a pinned-container build/smoke, not a WSL pass",
        "no Git tag, push, or GitHub Release was created",
        "binaries are not code-signed or notarized, and are not installable via any package manager (Homebrew/cargo/mise/apt/etc.) -- archives are extract-and-run only",
        "only x86_64 is built for either OS; no arm64/Apple Silicon-native or universal binary is produced or claimed",
        "the CI workflow's `update-test` job (.github/workflows/ci.yml) still performs a live end-to-end self-update test against real upstream agavra/tuicr GitHub releases; it was intentionally left unmodified per scope (see README.md) and is NOT a valid signal for this fork build, since this fork's `tuicr update` is hard-disabled and its --version format differs from what that job expects",
    ],
    "source": {
        "repository_remote": repo_remote,
        "branch": branch,
        "commit_full": sha_full,
        "commit_short": sha_short,
        "tree_dirty": False,
        "upstream_base_version": version,
        "fork_version": fork_version,
        "expected_reported_binary_version": expected_reported_version,
        "note": (
            "Fork identity is established BOTH in the binary itself (Cargo.toml "
            "version bumped to a semver prerelease tag, e.g. 0.19.1-offline-candidate.1, "
            "with the source commit short SHA embedded via build.rs and printed by "
            "`tuicr --version` as `tuicr <fork_version>+<commit_short>`) AND in this "
            "manifest. It does not impersonate an unmodified upstream release."
        ),
        "build_source_method": (
            "Both platform builds compile from a clean `git archive <commit_full> | "
            "tar -x` checkout of exactly the tracked source at this commit -- not the "
            "live working tree -- so no .tpatch files, .git internals, untracked "
            "session/audit/candidate data, or local env/config can be present in the "
            "build input regardless of what else exists untracked on the packaging host."
        ),
    },
    "toolchain": {
        "rustc_version": rustc_version,
        "cargo_version": cargo_version,
        "host_triple_macos": "x86_64-apple-darwin",
        "host_triple_linux_container": "x86_64-unknown-linux-gnu",
        "linux_build_image": linux_image,
        "linux_build_image_digest": linux_image_digest,
        "linux_build_platform_flag": "linux/amd64",
        "libgit2_provenance": (
            "git2's vendored-libgit2 feature is enabled in Cargo.toml, so both "
            "platforms compile libgit2 from the identical vendored C source bundled "
            "in the libgit2-sys crate version pinned in Cargo.lock, rather than "
            "linking whichever system libgit2 happens to be installed on the build "
            "host. This is an informational pin, not a security certification of "
            "that vendored copy."
        ),
    },
    "pre_build_checks": {
        "fmt_check": fmt_result,
        "clippy_all_targets_deny_warnings": clippy_result,
        "test_lib": {
            "result": test_result,
            "passed": int(test_passed or 0),
            "failed": int(test_failed or 0),
            "ignored": int(test_ignored or 0),
            "known_pre_existing_failures": [
                "vcs::git::libgit2::tests::should_discover_worktree_with_relativeworktrees_extension"
            ],
        },
        "secret_scan": {
            "method": (
                "Byte-aware (grep -a, never grep -I) explicit per-file scan of "
                "every regular file under the clean source checkout and each "
                "staged archive directory (including the compiled binary), run "
                "BEFORE tar packaging; fails packaging (die) on any match not in "
                "the narrow, exact-value SECRET_SCAN_ALLOWLIST. See "
                "docs/offline-candidate/SECRET-SCAN.md."
            ),
            "self_test": "passed (dummy binary with synthetic non-allowlisted token flagged; clean dummy binary not flagged; SECRET_SCAN_KNOWN_TOKEN_PREFIXES verified in sync with src/forge/mod.rs's KNOWN_TOKEN_PREFIXES and every prefix independently detected)",
            "report_file": "manifest/SECRET-SCAN.md",
            "design_doc": "docs/offline-candidate/SECRET-SCAN.md",
        },
    },
    "artifacts": artifacts,
    "dependency_snapshot": {
        "generated_by": "cargo metadata --locked --format-version 1",
        "file": "manifest/cargo-metadata.json",
        "license_summary_file": "manifest/THIRD-PARTY-LICENSES.md",
        "license_summary_disclaimer": "Audit aid only, not legal certification -- see the file's own header.",
    },
    "excluded_from_packaging": [
        "*.tpatch files",
        "the .git directory itself (build source is a git-archive export, not the working tree)",
        "credentials, tokens, and provider API keys",
        "user review-session data (~/.local/share/tuicr-offline-candidate, ~/Library/Application Support/tuicr-offline-candidate, and any real upstream tuicr/ data directory)",
        "candidate/research/audit data from sibling wayfinder worktrees",
        "built artifacts, target/ directories, and this script's own working directory",
    ],
    "known_risks": [
        {
            "id": "no-live-provider-validation",
            "severity": "high",
            "summary": (
                "This build was never exercised against a real GitHub, GitLab, "
                "Azure DevOps, Gitea, or Forgejo instance -- only offline fixture "
                "review/thread/dry-run CLI smoke tests were run. Provider-specific "
                "code paths that require live network calls are unvalidated."
            ),
            "mitigation": "See docs/offline-candidate/PROVIDER-CAPABILITIES.md.",
        },
        {
            "id": "not-a-wsl-pass",
            "severity": "medium",
            "summary": (
                "The Linux x86_64 artifact is built and smoke-tested in a pinned "
                "Docker container, not on real Ubuntu-under-WSL. Container behavior "
                "(filesystem, terminal/PTY handling, path conventions) can differ "
                "from WSL in ways this build has not exercised."
            ),
            "mitigation": "Treat as an explicit blocker; do not claim WSL support.",
        },
        {
            "id": "thread-only-rollback-hazard",
            "severity": "medium",
            "summary": (
                "This fork's sessions are written at schema version 1.4 "
                "(CURRENT_SESSION_VERSION), which can hold durable, provider-neutral "
                "PersistedThread data that upstream 0.19.1 does not understand. If a "
                "session containing thread-only state (no longer backed by legacy "
                "per-line Comment fields) is opened and saved by upstream 0.19.1, "
                "upstream can silently drop that thread-only state on save, since it "
                "only knows how to round-trip the legacy fields."
            ),
            "mitigation": (
                "Always back up reviews/ before installing/running upstream 0.19.1 "
                "against session files this fork created or touched. See "
                "MIGRATION.md's 'Rolling back to upstream 0.19.1' section."
            ),
        },
        {
            "id": "update_test_ci_job_not_fork_valid",
            "severity": "low",
            "summary": (
                ".github/workflows/ci.yml's `update-test` job performs a live "
                "end-to-end self-update test against real upstream agavra/tuicr "
                "GitHub releases. It was intentionally left unmodified (out of scope "
                "for this local packaging change per explicit instruction), but its "
                "results say nothing about this fork build: `tuicr update` is hard-"
                "disabled here, and --version's format differs from what that job's "
                "assertions expect."
            ),
            "mitigation": "Informational only; do not treat that CI job as a signal for this fork.",
        },
        {
            "id": "linux-binary-secret-scan-hash-allowlisted-false-positive",
            "severity": "low",
            "summary": (
                "The compiled Linux x86_64 tuicr binary contains one byte-sequence "
                "in its .rodata section that matches the gitlab-pat secret-scan "
                "pattern (glpat-...). It is deterministic and reproducible across "
                "independent rebuilds of the same commit/toolchain, but forensic "
                "analysis (27-byte match: 0 digits, 0 uppercase, only 13 distinct "
                "bytes, ~3.4 bits/char entropy vs ~5+ typical for a real random "
                "token; surrounding bytes are natural-language-like ASCII text; "
                "does not appear in any dependency source, build.rs OUT dir, or the "
                "full cargo registry used for the build; does not reproduce in an "
                "isolated minimal build of only the redact_secrets() prefix table; "
                "absent from the macOS x86_64 build entirely) rules out a real "
                "credential. Root cause is most likely dependency-embedded static "
                "string data unique to Linux-only compiled code paths, not "
                "definitively pinpointed to one exact source file. The literal "
                "value is intentionally NOT recorded anywhere (script or docs) -- "
                "only its SHA-256 digest is allowlisted, precisely because the "
                "matched bytes are secret-shaped even though they are not secret."
            ),
            "mitigation": (
                "Allowlisted by SHA-256 digest only (SECRET_SCAN_ALLOWLIST_SHA256 "
                "in scripts/package-offline-candidate.sh), never by plaintext or "
                "broad prefix/path suppression. See "
                "docs/offline-candidate/SECRET-SCAN.md for the full investigation."
            ),
        },
    ],
}

with open(out_path, "w") as f:
    json.dump(manifest, f, indent=2)
    f.write("\n")
PYEOF

log "done."
log "artifacts + checksums + manifest.json are in: $OUTPUT_DIR"
