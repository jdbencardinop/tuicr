# Release artifact security contract

Every native archive is scanned byte-for-byte before compression for common
GitHub, GitLab, AWS, Slack, and PEM private-key shapes. Non-allowlisted
matches abort packaging and prevent artifact upload.

The allowlist contains synthetic test fixtures plus two exact SHA-256 digests
for investigated compiled-binary false positives. The matched bytes are not
stored in source or logs:

| Target | Pattern | Match properties | SHA-256 | Evidence |
| --- | --- | --- | --- | --- |
| Linux x86_64 | GitLab PAT | documented in the archived candidate investigation | `861868d6e0246f776b867c651da9519f5d398cee945fe26ae2f0e890dcf64032` | `docs/offline-candidate/SECRET-SCAN.md` |
| Linux arm64 | GitHub other-prefix PAT | 56 bytes, 24 unique characters | `b693954df023f3cd9a2c073a2821437acd59a1ab6623229872259262a488b726` | Two independent native builds of commit `720e4a9` produced the exact digest; Linux x86_64 and both macOS targets had no corresponding match |

The Linux arm64 job has read-only repository permission and passes no provider
credential to Cargo or the packager. Its build inputs are public source,
locked public dependencies, the target toolchain, and the public source SHA.
The architecture-specific deterministic sequence is therefore classified as
compiled byte adjacency, although the exact linker/source-byte boundary has
not been isolated.

Allowlisting is by complete matched-value digest only. A changed byte, another
target, or another token-shaped value remains blocking.
