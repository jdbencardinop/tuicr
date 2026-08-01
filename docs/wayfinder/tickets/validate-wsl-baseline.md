---
id: validate-wsl-baseline
title: Validate the upstream WSL baseline
type: task
mode: HITL
status: blocked
owner: copilot
blocked_by: []
---

## Question

Run unmodified upstream Tuicr and the common review fixture inside actual
Ubuntu under WSL before fork work can hide a platform-specific problem.

## Completion

Install, startup, navigation, comment persistence, clipboard/stdout,
browser/editor launch, and `gh`/`glab` credential behavior recorded with the
same run template used on macOS.

## Status

**Still blocked — no real WSL runtime has ever been reached.** Development so
far has happened on a Darwin host with no `wsl` executable, no
`/proc/version`, and no Windows/WSL runtime. A Linux Docker container is not
an acceptable substitute: WSL-specific behavior (`gh`/`glab` credential
helpers, clipboard integration, browser/editor launch via the Windows host,
and WSL-specific filesystem/interop quirks) cannot be reproduced by spoofing
`WSL_DISTRO_NAME`/`WSL_INTEROP` inside a plain container.

A validation run needs, at minimum: install, startup, navigation, comment
persistence, clipboard/stdout, browser/editor launch, and `gh`/`glab`
credential behavior, recorded with the same run template already used on
macOS (see `docs/fork/AGENT-WORKFLOW.md` for the build/lint/test commands
that also apply on WSL). The pinned upstream binary
(`v0.19.1`, `f92502bfbbd172ceb3c16c3dfd1348b52e142be5`) needs glibc ≥ 2.39
(works on `ubuntu:24.04`, fails clearly on `ubuntu:22.04`); an older LTS run
should fall back to `cargo install --locked --git
https://github.com/agavra/tuicr --tag v0.19.1 tuicr`.

## Unblock condition

Run the install/startup/navigation/persistence/clipboard/credential checks
above from an interactive shell inside a real Ubuntu WSL distro (via
`wsl.exe`/Windows Terminal, not a container), and record the pass/fail
results and any WSL-specific deviations directly in this ticket's Resolution
section once run.
