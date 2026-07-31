# Secret scan design (offline-candidate packaging)

This document describes `scripts/package-offline-candidate.sh`'s built-in
secret scanner: what it does, its exact allowlist, and its self-test. It is
an **audit aid recorded alongside each packaging run's generated
`manifest/SECRET-SCAN.md` report** -- it is not a certification that no
secret exists anywhere in the repository or artifacts, and it does not
replace independent security review. No new scanning tool/dependency is
added; this is a small `grep`-based pattern scan already achievable with
tools present in the packaging environment.

## What is scanned, and how

- **Every regular file** under the scan root is scanned -- the clean
  `git archive` source checkout used for both platform builds, and,
  separately, the fully staged per-platform archive directory (binary +
  `LICENSE` + offline-candidate docs + bundled config + import script)
  **before** that directory is `tar`'d into the shipped `.tar.gz`.
- Files are enumerated explicitly via `find "$scan_root" -type f`, not
  discovered implicitly by a recursive `grep -R`, so every file that will
  end up in an archive -- including the compiled `tuicr` binary -- is
  guaranteed to be scanned. The compiled binary is scanned every time an
  archive is staged, including for the produced release artifacts.
- Matching uses `grep -aoE` (`-a`, byte-aware / treat-as-text), **never**
  `grep -I` or plain `grep` without `-a`. Earlier versions of this script
  used `grep -RIlE`, whose `-I` flag makes grep guess a file is binary from
  its content and silently skip it without matching -- which meant the
  compiled binary was never actually scanned even though the report implied
  full staged-archive coverage. `-a` scans every file's raw bytes uniformly,
  whether it is text or binary, so there is no silent skip and no
  binary/text distinction in scan coverage to misrepresent.
- Secret-shaped patterns checked (see `SECRET_SCAN_PATTERNS` /
  `SECRET_SCAN_PATTERN_IDS` in the script for the exact regexes): GitHub
  classic PAT (`ghp_`), GitHub other prefixes (`gho_`/`ghs_`/`ghu_`/`ghe_`),
  GitHub fine-grained PAT (`github_pat_`), GitLab PAT (`glpat-`), AWS access
  key ID (`AKIA...`), Slack token (`xox[baprs]-...`), PEM private key
  header. The GitHub/GitLab patterns are open-ended (`{36,}`, `{20,}`, not a
  fixed `{36}`/`{20}`) so a match captures a full contiguous token rather
  than silently truncating a longer real value to a prefix, which would
  otherwise make exact-value allowlist comparison unreliable.

## Report contents: filenames + categories only, never values

The generated `manifest/SECRET-SCAN.md` report (and this design doc) never
records a matched secret's actual value -- only the relative file path and
the matched pattern's category name (e.g. `github-pat-classic`). This is
deliberate: a scan report is itself a document that could end up committed,
shared, or archived, and must not become a place secrets leak to even when
scanning finds one.

## Allowlist: narrow, exact-value only -- never path/prefix/directory

Some of this repository's own tracked test source deliberately contains
secret-**shaped** literals as test fixtures (e.g. testing that
`redact_secrets` scrubs a GitHub/GitLab token, or that CLI/log output never
echoes a raw PAT back). These are not real credentials, but they do match
the patterns above by construction. To avoid failing packaging on its own
known-safe fixtures while still failing hard on anything unexpected, the
scanner's allowlist (`SECRET_SCAN_ALLOWLIST` in the script) is:

- **A list of complete, literal, exact matched-value strings** -- each
  entry must equal a captured match byte-for-byte to be treated as
  allowlisted.
- **Never** a path, directory, filename glob, or value *prefix* -- a
  slightly different (longer, shorter, or otherwise distinct) secret-shaped
  value in the exact same file/line would still fail packaging.

Current allowlist entries and their provenance (all synthetic,
`SENTINEL`/alphabet-pattern test fixtures, not real credentials):

| Value | Source (file:line at the time this doc was written) |
| --- | --- |
| `ghp_SENTINEL0123456789abcdefABCDEF01234567` | `src/forge/github/gh.rs:2494`, `src/forge/github/gh.rs:2623`, `src/forge/integration_tests.rs` (redaction test fixture) |
| `glpat-SENTINEL0123456789abcdefABCDEF` | `src/forge/gitlab/glab.rs:2428` |
| `glpat-SENTINEL0123456789abcdef` | `src/forge/gitlab/glab.rs:2371` (distinct, shorter fixture used in a separate stderr-scrubbing test) |
| `glpat-XyZ_0123456789abcdef` | `src/forge/mod.rs:169` |
| `glpat-ABCDEFGHIJKLMNOPQRST` | `src/forge/integration_tests.rs` (redaction test fixture) |

If tracked source ever adds a *new* secret-shaped test fixture, packaging
will fail until a maintainer reviews the new value, confirms it is not a
real credential, and adds its exact literal to `SECRET_SCAN_ALLOWLIST` in
`scripts/package-offline-candidate.sh` (updating this table too). This is
intentional friction -- an unreviewed new match should never silently pass.

## Self-test: proving the scanner works before trusting it

Before any real scan runs, `self_test_secret_scan()` (in the packaging
script) builds two throwaway dummy files under a scratch directory:

- A "clean" dummy file: a short run of non-text bytes with no secret-shaped
  content.
- A "dirty" dummy file: the exact same non-text byte prefix, plus a
  deliberately synthetic, non-allowlisted secret-shaped token, assembled
  at runtime from two separate fragments (a `ghp_`-prefix variable and a
  36-character alphanumeric body variable, concatenated only when the
  script runs) so the full contiguous value never appears as a literal
  anywhere in this script's or this doc's own tracked text -- otherwise
  the real "clean source checkout" scan below would flag this very
  file/doc. Not a real credential and not in `SECRET_SCAN_ALLOWLIST`.

It then asserts, via the same `_secret_scan_detect` code path used for the
real scans: the clean file produces **zero** hits, and the dirty file
produces **at least one** hit. If either assertion fails -- a false
positive on the clean file, or a false negative (binary-skip regression) on
the dirty file -- packaging aborts immediately (`die`), because a
scanning mechanism that hasn't been shown to work should not be trusted to
gate a release.

## Fail-closed behavior

`run_secret_scan()` calls `die` on any non-allowlisted match, for both the
clean-source-checkout scan and each platform's pre-tar staged-archive scan.
Packaging cannot produce a checksummed `.tar.gz` archive if a real,
non-allowlisted secret-shaped value is present anywhere in the files that
would go into it -- the scan runs and must pass **before** `tar` is
invoked, not after.
