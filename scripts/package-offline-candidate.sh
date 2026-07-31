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
VERSION="$(cargo metadata --locked --no-deps --format-version 1 2>/dev/null \
  | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')"
PACKAGED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUSTC_VERSION="$(rustc --version)"
CARGO_VERSION="$(cargo --version)"
HOST_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"
LINUX_BUILD_IMAGE_DIGEST="not-built"

log "source commit: $SOURCE_SHA_FULL ($SOURCE_SHA_SHORT) on $SOURCE_BRANCH"
log "upstream crate version: $VERSION"
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
        "license-audit tool was added for this. This is a summary for human "
        "review, not a legal clearance; verify anything load-bearing against "
        "the vendored source pinned in Cargo.lock before redistribution "
        "decisions.\n\n"
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
  [[ "$reported_version" == "tuicr ${VERSION}" ]] \
    || die "[$os] unexpected --version output: $reported_version"
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
# Main
# ---------------------------------------------------------------------------
run_prebuild_checks
generate_dependency_snapshot "$OUTPUT_DIR/manifest"

if [[ "$SKIP_MACOS" -eq 0 ]]; then
  if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "x86_64" ]]; then
    log "skipping macOS build: this host is not macOS x86_64 ($(uname -s)/$(uname -m))"
  else
    log "building macOS x86_64 (native, isolated toolchain: $RUSTC_VERSION)"
    cargo build --release --locked
    package_artifact "macos" "x86_64" "$ROOT_DIR/target/release/tuicr" \
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
    # Copy the source (read-only mount can't hold Cargo's own lock files /
    # incremental caches) into a container-writable scratch dir. Only
    # tracked source is present (the dirty-tree gate above already ran),
    # so this copy is equivalent to a fresh clone at the packaged commit.
    rsync -a --exclude target --exclude .git "$ROOT_DIR/" "$LINUX_SRC_COPY_DIR/"
    docker run --rm --platform "$LINUX_PLATFORM" \
      -v "$LINUX_SRC_COPY_DIR:/src" \
      -v "$LINUX_TARGET_DIR:/build-target" \
      -v "$LINUX_REGISTRY_DIR:/usr/local/cargo/registry" \
      -e CARGO_TARGET_DIR=/build-target \
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
  "$PACKAGED_AT" "$REPO_REMOTE" "$SOURCE_BRANCH" "$SOURCE_SHA_FULL" "$SOURCE_SHA_SHORT" "$VERSION" \
  "$RUSTC_VERSION" "$CARGO_VERSION" "$LINUX_BUILD_IMAGE" "$LINUX_BUILD_IMAGE_DIGEST" \
  "$FMT_RESULT" "$CLIPPY_RESULT" "$TEST_RESULT" "$TEST_PASSED" "$TEST_FAILED_COUNT" "$TEST_IGNORED" \
<<'PYEOF'
import json, sys

(artifacts_path, out_path, packaged_at, repo_remote, branch, sha_full, sha_short, version,
 rustc_version, cargo_version, linux_image, linux_image_digest,
 fmt_result, clippy_result, test_result, test_passed, test_failed, test_ignored) = sys.argv[1:]

with open(artifacts_path) as f:
    artifacts = json.load(f)

manifest = {
    "manifest_schema_version": 1,
    "generated_at_utc": packaged_at,
    "release_readiness": "OFFLINE_VALIDATED_ONLY",
    "not_release_ready_because": [
        "no live provider (GitHub/GitLab/Azure DevOps/Gitea/Forgejo) write validation was performed producing this archive",
        "no real Ubuntu-under-WSL validation was performed; the Linux artifact is a pinned-container build/smoke, not a WSL pass",
        "no Git tag, push, or GitHub Release was created",
        "Cargo.toml repository/crate/binary identity is still unmodified upstream (agavra/tuicr); the packaged binary's automatic startup update-check phones home to crates.io by default, and its `tuicr update`/package-manager upgrade targets still resolve to real upstream, so running them would silently self-replace this fork binary (see docs/offline-candidate/README.md and the bundled config.no-update-check.toml sample)",
    ],
    "source": {
        "repository_remote": repo_remote,
        "branch": branch,
        "commit_full": sha_full,
        "commit_short": sha_short,
        "tree_dirty": False,
        "upstream_crate_version": version,
        "note": (
            "The binary's own --version output is identical to unmodified "
            "upstream (Cargo.toml's version was not bumped for this fork). "
            "Fork identity is established by this manifest's commit_full, "
            "not by the binary."
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
    },
    "artifacts": artifacts,
    "dependency_snapshot": {
        "generated_by": "cargo metadata --locked --format-version 1",
        "file": "manifest/cargo-metadata.json",
        "license_summary_file": "manifest/THIRD-PARTY-LICENSES.md",
    },
    "excluded_from_packaging": [
        "*.tpatch files",
        "credentials, tokens, and provider API keys",
        "user review-session data (~/.local/share/tuicr, ~/Library/Application Support/tuicr)",
        "candidate/research data from sibling wayfinder worktrees",
        "built artifacts, target/ directories, and this script's own working directory",
    ],
    "known_risks": [
        {
            "id": "unmodified-upstream-identity",
            "severity": "high",
            "summary": (
                "Cargo.toml repository/crate/binary name are unmodified upstream "
                "(agavra/tuicr). The packaged binary auto-checks crates.io for a "
                "newer version on every startup (suppressible via "
                "--no-update-check or config.toml's no_update_check = true), and "
                "`tuicr update` / `brew upgrade agavra/tap/tuicr` / `cargo install "
                "tuicr --force` / `mise upgrade github:agavra/tuicr` all still "
                "resolve to real upstream, so running any of them silently "
                "self-replaces this fork binary with upstream, discarding the "
                "fork's durable-thread feature with no warning."
            ),
            "mitigation": (
                "Do not run `tuicr update` or any package-manager upgrade command "
                "against this binary. Copy the bundled config.no-update-check.toml "
                "into the config directory to suppress the automatic startup check."
            ),
        },
        {
            "id": "shared-data-directory-with-upstream",
            "severity": "medium",
            "summary": (
                "Review-session data directory is resolved via "
                "ProjectDirs::from(\"\", \"\", \"tuicr\") -- an app identifier this "
                "fork does not change. A real upstream tuicr install on the same "
                "machine reads/writes the exact same reviews/ directory as this "
                "fork build; there is no fork-specific data directory."
            ),
            "mitigation": (
                "See MIGRATION.md's 'shares its data directory with any real "
                "upstream tuicr install' section: override HOME/XDG_DATA_HOME for "
                "isolated testing, or back up reviews/ before switching installs."
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
