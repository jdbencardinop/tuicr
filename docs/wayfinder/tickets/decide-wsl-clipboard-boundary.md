---
id: decide-wsl-clipboard-boundary
title: Decide the WSL clipboard release boundary
type: grilling
mode: HITL
status: in-progress
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
