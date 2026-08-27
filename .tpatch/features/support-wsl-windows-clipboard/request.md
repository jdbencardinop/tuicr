# Feature Request: On Linux under WSL with Windows interoperability available, copy review exports through clip.exe before tmux/OSC 52, X11, Wayland, or arboard. Preserve existing native macOS, native Linux, SSH, Zellij, and PowerShell-host behavior, fall back when clip.exe is unavailable or fails, and add regression tests for platform detection, routing order, exact stdin bytes, failure fallback, and non-WSL isolation.

**Slug**: `support-wsl-windows-clipboard`
**Created**: 2026-08-27T21:11:18Z

## Description

On Linux under WSL with Windows interoperability available, copy review exports through clip.exe before tmux/OSC 52, X11, Wayland, or arboard. Preserve existing native macOS, native Linux, SSH, Zellij, and PowerShell-host behavior, fall back when clip.exe is unavailable or fails, and add regression tests for platform detection, routing order, exact stdin bytes, failure fallback, and non-WSL isolation.

Live WSL probing showed that clip.exe corrupts raw UTF-8 through the Windows console code page. Encode only this bridge as UTF-16LE, accept Windows Clipboard CRLF normalization, and prove multiline Japanese plus a supplementary-plane emoji through PowerShell Get-Clipboard.
