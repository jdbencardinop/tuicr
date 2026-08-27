# Analysis: support-wsl-windows-clipboard

## Summary

Tuicr does not distinguish WSL from native Linux. Outside tmux it can accept
an `arboard`/WSLg success that does not update the Windows clipboard; inside
tmux it prefers OSC 52, which cannot reach an unattached Windows Terminal
client. WSL already exposes the verified `clip.exe` bridge through Windows
interop.

## Compatibility

**Status:** compatible with an explicit platform-and-environment gate.

The new bridge must run only for a Linux binary with non-empty
`WSL_INTEROP`. Native Linux must retain OSC 52, Wayland/X11, and `arboard`;
macOS must continue preferring `pbcopy`; native Windows, including
PowerShell-hosted execution, must continue using the system clipboard.

## Affected Areas

- `src/output/markdown.rs`: clipboard route selection and focused tests.
- Clipboard documentation, Wayfinder decision, patch index, and handoff.

## Acceptance Criteria

1. WSL with active interop tries `clip.exe` before tmux/OSC 52 and Linux
   clipboard backends.
2. A missing or failing `clip.exe` falls through to the prior route.
3. The bridge receives the exact export bytes on stdin.
4. Native Linux, macOS, and native Windows never select the WSL bridge.
5. Existing clipboard behavior and the supported library suite remain green.
6. A real WSL TUI export updates and round-trips through Windows Clipboard.

## Unresolved Questions

- None. The user selected native WSL integration rather than documenting the
  existing workaround as the release boundary.

## Live encoding finding

The first live attempt proved that `clip.exe` interprets raw UTF-8 through a
Windows console code page and corrupts non-ASCII text. UTF-16LE stdin preserves
the complete Unicode fixture; Windows Clipboard canonically normalizes line
endings to CRLF. The implementation therefore encodes only the `clip.exe`
route as UTF-16LE while every existing clipboard command still receives the
original UTF-8 bytes.
