//! Embeds the source commit SHA this offline-candidate build was compiled
//! from into `TUICR_OFFLINE_CANDIDATE_SHA`, which `src/cli.rs` appends to
//! `--version` output (alongside the `-offline-candidate.N` prerelease tag
//! already in `Cargo.toml`'s `version`). This lets anyone confirm a packaged
//! binary's exact provenance without trusting an unpinned `--version`
//! string.
//!
//! `scripts/package-offline-candidate.sh` builds from a clean `git archive`
//! checkout (no `.git` directory), so it exports
//! `TUICR_OFFLINE_CANDIDATE_SHA` explicitly before invoking `cargo build`;
//! that value takes priority here. Plain local development builds (with a
//! `.git` directory present) fall back to `git rev-parse` directly.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=TUICR_OFFLINE_CANDIDATE_SHA");
    println!("cargo:rerun-if-changed=.git/HEAD");

    let sha = std::env::var("TUICR_OFFLINE_CANDIDATE_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_short_sha)
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=TUICR_OFFLINE_CANDIDATE_SHA={sha}");
}

fn git_short_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8(output.stdout).ok()?;
    let sha = sha.trim();
    (!sha.is_empty()).then(|| sha.to_string())
}
