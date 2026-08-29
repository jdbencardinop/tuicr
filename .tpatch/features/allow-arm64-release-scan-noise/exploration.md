# Exploration: allow-arm64-release-scan-noise

The only code change is the exact-digest allowlist in
`scripts/package-release-artifact.sh`. Documentation belongs in
`docs/release/SECURITY.md`, and the packager must copy that file into the
staged archive. No scanner pattern, provider code, credential source, or
workflow permission changes.
