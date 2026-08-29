# Install the maintained fork

The primary distribution is a checksummed GitHub release archive for one of:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

Verify the adjacent `.sha256` file before extracting, then place `tuicr` on
`PATH`. macOS archives are ad-hoc signed unless the release notes explicitly
state that Developer ID signing and Apple notarization passed.

The package-manager-capable source install is Cargo from an exact tag:

```bash
cargo install --git https://github.com/jdbencardinop/tuicr \
  --tag <release-tag> --locked
```

The fork does not publish the upstream-owned `tuicr` crate to crates.io.
Automatic update checks and `tuicr update` remain disabled; upgrade by
installing a newer verified tag and rollback by reinstalling an older one.
The inherited Nix flake is not a first-release channel because its naersk
fixed-output dependency fetch repeatedly received crates.io HTTP 403 in CI.
