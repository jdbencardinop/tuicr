# Analysis: defer-nix-release-channel

## Summary

The inherited naersk Nix build failed on three consecutive GitHub-hosted runs
because crates.io returned HTTP 403 for different locked dependencies. Cargo
source installation passed. Nix therefore cannot be claimed as a validated
first-release distribution channel.

## Decision

Keep the flake for future repair and manual investigation, but remove Nix from
the first-release install contract. Restrict the legacy workflow to manual
dispatch so an unrelated external fetch failure is not a branch/release gate.
Cargo git from an exact fork tag is the validated package-manager path.
