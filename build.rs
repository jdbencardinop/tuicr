//! Embeds the source commit SHA into `TUICR_BUILD_SHA`, which `src/cli.rs`
//! appends to `--version`. This lets anyone confirm a packaged binary's exact
//! provenance without trusting an unpinned version string.
//!
//! Release packaging can build from a source archive without `.git`, so it
//! exports `TUICR_BUILD_SHA` explicitly. Plain development builds fall back
//! to `git rev-parse`.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=TUICR_BUILD_SHA");
    println!("cargo:rerun-if-changed=.git/HEAD");

    let sha = std::env::var("TUICR_BUILD_SHA")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(git_short_sha)
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=TUICR_BUILD_SHA={sha}");
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
