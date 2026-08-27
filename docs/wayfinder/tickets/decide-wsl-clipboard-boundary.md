---
id: decide-wsl-clipboard-boundary
title: Decide the WSL clipboard release boundary
type: grilling
mode: HITL
status: closed
owner: copilot
blocked_by: []
---

## Question

Must the fork's TUI clipboard action write directly to the Windows clipboard
for the first release, or is the verified structured-stdout/direct-`clip.exe`
workflow an acceptable documented boundary?

## Completion

Either the release contract explicitly accepts and teaches the workaround, or
the TUI clipboard path is fixed and verified through an attached Windows
Terminal/tmux client. The decision and evidence are recorded here before the
release ticket closes.

## Evidence

The WSL run proved direct `clip.exe` round-trip and structured stdout export,
but the TUI clipboard action did not replace the Windows clipboard.

The 2026-08-27 implementation rerun passed direct library, detached tmux, and
real-TUI `y` exports through Windows Clipboard with multiline Japanese and a
supplementary-plane emoji. Sanitized evidence:
`../../../../artifacts/validation/2026-08-27-wsl-windows-clipboard/`.

## Resolution

The first release requires native Windows Clipboard delivery from WSL rather
than teaching `--stdout | clip.exe` as the primary boundary. Linux builds with
active `WSL_INTEROP` now invoke `clip.exe` before tmux/OSC 52, Wayland/X11, or
`arboard`; a missing or failed bridge falls through to the prior routes.

`clip.exe` interprets raw UTF-8 through a Windows console code page, so its
route encodes text as UTF-16LE. Generic Linux commands continue receiving
unchanged UTF-8, macOS still prefers `pbcopy`, and native Windows/PowerShell
still uses `arboard`. Windows Clipboard's canonical CRLF normalization is
accepted.
