# Exploration: support-wsl-windows-clipboard

## Current flow

`copy_text_to_clipboard` in `src/output/markdown.rs` prefers `pbcopy` on
macOS, then OSC 52 for tmux/SSH/Zellij, then `wl-copy`/`xclip`, then
`arboard`, and finally OSC 52. It has no WSL branch.

On the validation host, `WSL_INTEROP`, `WT_SESSION`, `WAYLAND_DISPLAY`, and
`DISPLAY` are present, but `XDG_SESSION_TYPE` is unset. `clip.exe`,
PowerShell, and `xclip` are available; `wl-copy` is not. The existing Linux
subprocess branch is therefore skipped outside tmux. In detached tmux,
`tmux load-buffer -w` updates the tmux buffer without an attached terminal
that can forward OSC 52 to Windows Clipboard.

## Minimal changeset

- `src/output/markdown.rs`
  - Snapshot only the environment flags needed for routing.
  - Identify the compiled target independently from runtime environment.
  - Prepend a `clip.exe` command route only for Linux plus active
    `WSL_INTEROP`.
  - Keep command execution shell-free and pass review text only on stdin.
  - Unit-test foreign platform combinations through the pure planner.
- `CHANGELOG.md`
  - Record the WSL clipboard fix.

No dependency, configuration, CLI, persistence, forge, or review-format
change is required.
