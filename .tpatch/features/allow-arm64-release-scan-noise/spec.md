# Specification: allow-arm64-release-scan-noise

1. Add the Linux arm64 matched-value SHA-256 to the narrow digest allowlist.
2. Preserve the prior Linux x86_64 digest and literal synthetic fixtures.
3. Document target, pattern class, length, diversity, deterministic rebuild,
   build-input boundary, and residual root-cause limitation.
4. Include the security contract in every packaged archive.
5. Rerun the complete native matrix and aggregate checksum gate.
