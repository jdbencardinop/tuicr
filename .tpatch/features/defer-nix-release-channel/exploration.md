# Exploration: defer-nix-release-channel

The change is limited to release documentation and
`.github/workflows/build_nix.yml`. The flake and dependency lock remain
untouched because the observed failures moved between `allocator-api2` and
`aho-corasick` while direct crates.io downloads remained available, indicating
an external fixed-output fetch boundary rather than a package compile error.
