# Specification: support-wsl-windows-clipboard

## Acceptance Criteria

1. Clipboard route selection is deterministic from an injected target
   platform and environment snapshot.
2. Linux plus non-empty `WSL_INTEROP` prepends `clip.exe`.
3. WSL plus tmux still tries `clip.exe` before OSC 52.
4. Empty/missing `WSL_INTEROP`, macOS, and native Windows do not select
   `clip.exe`.
5. Generic command execution writes multiline Unicode UTF-8 bytes unchanged.
   The `clip.exe` bridge writes UTF-16LE so Windows preserves Unicode; Windows
   Clipboard CRLF normalization is accepted. Spawn, write, or nonzero-exit
   failures are treated as unavailable.
6. Regression tests cover route isolation and fallback ordering without
   requiring Windows executables on non-Windows CI.

## Implementation Plan

1. Introduce private target, environment, and route types in
   `src/output/markdown.rs`.
2. Build a pure ordered route list and execute it through existing clipboard
   helpers.
3. Add route-planning, failed-bridge fallback, and exact-stdin tests.
4. Validate on WSL through the real TUI and Windows Clipboard.
5. Record, verify, and land the patch through `tpatch`; close the Wayfinder
   ticket and update release-facing documentation.
