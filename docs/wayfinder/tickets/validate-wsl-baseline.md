---
id: validate-wsl-baseline
title: Validate the upstream WSL baseline
type: task
mode: HITL
status: closed
owner: copilot
blocked_by: []
---

## Question

Run unmodified upstream Tuicr and the common review fixture inside actual
Ubuntu under WSL before fork work can hide a platform-specific problem.

## Completion

Install, startup, navigation, comment persistence, clipboard/stdout,
browser/editor launch, and in-scope provider credential behavior recorded
with the same run template used on macOS.

## Resolution

**Closed 2026-08-11 — pass with caveats.** The validation ran in real WSL2,
not Docker: Ubuntu 24.04.4 x86_64, glibc 2.39, kernel
`6.6.87.2-microsoft-standard-WSL2`, WSL 2.5.9.0/WSLg 1.0.66, Windows
10.0.26200.8893, and Git 2.43.0.

The exact sources were upstream `v0.19.1` commit
`f92502bfbbd172ceb3c16c3dfd1348b52e142be5` and fork commit
`c2a75543327b996f65c277cacaa5289bb214cc36` (reported as
`tuicr 0.19.1-offline-candidate.1+c2a7554`). The upstream release archive's
recorded SHA-256
`67addacc28ee9c6d1ce84220954f1ff0c6a82b374822142dd8cb615b3d425ec5`
verified before execution.

| Surface | Observed result |
|---|---|
| Build | Upstream and fork passed `cargo fmt --all --check`, clippy with warnings denied, and locked release builds on native WSL. |
| Tests | All 1,181 upstream and 1,642 fork tests supported by Ubuntu's Git passed. The fork's scoped Azure suite passed 60 tests. |
| Startup | The 1,000-line fixture rendered in 625 ms upstream and 442 ms in the fork under a real tmux PTY, below the five-second gate. |
| Navigation | Cursor, search, file, hunk, and comment navigation changed the expected viewport in both builds. |
| Persistence | Review/file/new-line/old-line/range comments, one TUI-entered comment, and reviewed state persisted. Fork thread/reply/resolve state and its separate data directory also persisted. |
| Output | Structured stdout export passed in both builds. A direct `clip.exe` write/read round trip passed. |
| Launch | TUI editor handoff passed; Windows VS Code and PowerShell browser launch returned success. |
| Credentials | Authenticated `gh` API/PR reads passed. Azure CLI auth, Azure DevOps token acquisition/defaults/project read, 60 adapter tests, and an eight-operation local dry-run passed. GitLab is out of scope for this WSL baseline and deferred to the self-hosted target in `provision-provider-sandboxes.md`. |

Five caveats are now observed facts:

- After inserting five lines, a comment anchored to line 42 stayed at numeric
  line 42 and rendered on `policy_rule_037`; neither build followed
  `policy_rule_042` to line 47 or surfaced the anchor as stale.
- TUI clipboard export did not replace the Windows clipboard outside tmux,
  and detached tmux could only populate its terminal buffer. `--stdout` and
  direct `clip.exe` are working WSL workarounds.
- Ubuntu's Git 2.43 cannot exercise
  `should_discover_worktree_with_relativeworktrees_extension` or
  `default_preference_routes_reftable_repo_to_cli`; rerunning with only those
  two Git-version fixtures skipped passed every remaining test. Git 2.48 is
  the oldest release that supports both exercised features.
- The pinned upstream `v0.19.1` Linux binary requires glibc 2.39. Ubuntu 24.04
  satisfies that requirement; an Ubuntu 22.04 WSL baseline must build from
  source with Cargo rather than execute the published binary.
- `wslview` returns zero but prints that interoperability is disabled because
  its expected binfmt marker is absent. Windows executables work, and
  PowerShell `Start-Process` is the verified browser path.

No remote comments, reviews, threads, votes, or other provider mutations were
performed. Live mutation parity remains owned by the sandbox tickets.
