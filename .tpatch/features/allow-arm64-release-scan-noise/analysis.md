# Analysis: allow-arm64-release-scan-noise

## Summary

The native Linux arm64 binary reproducibly contains one 56-byte sequence that
matches the scanner's `gh[oesur]_` token shape. Two independent builds of
commit `720e4a9` produced the same matched-value SHA-256
`b693954df023f3cd9a2c073a2821437acd59a1ab6623229872259262a488b726`.
The other three native targets passed unchanged.

## Security assessment

The candidate job has read-only repository permission and passes no provider
credential to Cargo or the packager. The only explicit build input is the
public source SHA. The binary is built from public source and a locked public
dependency graph. This makes the deterministic architecture-specific match
compiled byte adjacency rather than a captured live credential.

Allow only the exact matched-value digest. Do not log or commit the
secret-shaped bytes, weaken the pattern, allow a prefix, or exempt the arm64
binary as a whole.

## Acceptance criteria

1. The exact digest is documented and allowlisted.
2. Any one-byte variation remains blocking.
3. Every platform continues through the same byte-aware scanner.
4. The security note is included in release archives.
