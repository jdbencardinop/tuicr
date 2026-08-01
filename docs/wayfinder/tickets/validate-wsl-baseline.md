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

**Still blocked — no real WSL runtime has ever been reached.** This Darwin
host has no `wsl` executable, no `/proc/version`, and no Windows/WSL runtime.
A Linux Docker container is explicitly rejected as a substitute (confirmed by
running the harness inside `ubuntu:22.04`/`ubuntu:24.04` containers, including
with spoofed `WSL_DISTRO_NAME`/`WSL_INTEROP`, and observing it correctly
refuse every time).

A rerunnable, self-gating harness and checklist are ready and were harness
-verified (not WSL-verified) inside Docker with the real-WSL gate
intentionally bypassed for that test only:

- `scripts/validate-wsl-baseline.sh` — refuses unless four independent real
  -WSL signals agree, generates/validates the common fixture, resolves a
  pinned `v0.19.1` (`f92502bfbbd172ceb3c16c3dfd1348b52e142be5`) Tuicr binary
  (operator-supplied or downloaded+sha256-verified), runs a bounded pty
  startup probe plus a CLI review-session smoke test, captures `gh`/`glab`
  credential/clipboard/editor/browser/network evidence, and writes results to
  gitignored `artifacts/raw/wsl-baseline-<run-id>/`.
- `docs/evaluation/wsl-baseline-checklist.md` (research workspace) — the exact
  run-metadata / automated-results / manual-HITL / classification format to
  fill in during a real run.

Known environment note: the pinned Linux release binary needs glibc ≥ 2.39
(works on `ubuntu:24.04`, fails clearly on `ubuntu:22.04`); an older LTS run
should fall back to `cargo install --locked --git
https://github.com/agavra/tuicr --tag v0.19.1 tuicr`.

## Unblock condition

Run `scripts/validate-wsl-baseline.sh` from an interactive shell inside a real
Ubuntu WSL distro (via `wsl.exe`/Windows Terminal — the script self-verifies
this is not a container), fill in the checklist's manual section, and link the
resulting `artifacts/raw/wsl-baseline-<run-id>/` from
`docs/findings/benchmarks/wsl-evidence-gap.md` in the research workspace.

Full narrative, including the harness's own bug-fix history, is in the
research workspace's
`docs/follow-on-map/tickets/03-validate-wsl-baseline.md`.
