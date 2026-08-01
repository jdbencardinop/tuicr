# Local `tpatch` committed-range verifier fix

This documents a locally-verified fix to the `tpatch` CLI itself (not to this
fork's source), needed until an upstream `tpatch` release contains equivalent
behavior.

## Problem

`tpatch verify` (the V7/V8 closure-replay checks) always rooted its replay
shadow at live HEAD. For a **committed-range** capture (`tpatch record
--auto`, `--from`/`--to`, or `--commit-range`), the feature's own commits land
on the branch between the recorded `status.apply.base_commit` and HEAD, so a
HEAD-rooted shadow already contains the feature's change. Both replaying the
apply recipe (V7) and running `git apply --check` against the canonical patch
(V8) then double-apply it, and V8 fails because the patch's context no
longer matches a tree that already has the change.

The `tpatch` binary built from v0.11.3 (release commit `84a2f88`) has this
bug. The fix landed as commit `d8e3e15` ("fix(verify): root V7/V8 shadow at
recorded base for committed-range captures") in
[`tesseracode/tesserapatch`](https://github.com/tesseracode/tesserapatch),
one commit after the `84a2f88` v0.11.3 release tag.

## When this matters here

Only for `tpatch verify`/`tpatch record` runs against a **committed-range**
capture in this repo's `.tpatch/` workspace (bootstrapped at commit
`24376a6`, see `.tpatch/README.md`'s "Committed-range verification caveat").
Working-tree captures (`tpatch record <slug>` with no range) are unaffected
by this bug and need no workaround.

## Fix

[`patches/d8e3e15-tpatch-verify-committed-range-base.patch`](patches/d8e3e15-tpatch-verify-committed-range-base.patch)
is a clean `git format-patch -1` export of commit `d8e3e15`, applying on top
of `tesseracode/tesserapatch` at `84a2f88` (v0.11.3). It touches only that
tool's own repository:

- `internal/workflow/verify.go` — roots the shadow at the recorded
  `base_commit` for committed-range captures instead of always using HEAD,
  and fails verify explicitly (never a silent HEAD fallback) when that base
  commit is empty, unresolvable, or unreachable.
- `internal/workflow/verify_committed_range_test.go` — four new regression
  tests.
- `CHANGELOG.md` — an `Unreleased` entry describing the fix.

## How to apply/build

```
git clone https://github.com/tesseracode/tesserapatch
cd tesserapatch
git checkout 84a2f88
git am /path/to/this/repo/docs/fork/patches/d8e3e15-tpatch-verify-committed-range-base.patch
go build -o tpatch ./cmd/tpatch
```

Use the resulting `tpatch` binary (or put it ahead of any other `tpatch` on
`PATH`, e.g. in place of a `~/go/bin/tpatch` built before 2026-07-30) whenever
running `tpatch verify`/`tpatch record` against a committed-range capture on
Windows/WSL or any other host until an upstream `tpatch` release contains
commit `d8e3e15` or equivalent behavior. Check `tpatch --version` /
`git -C <tpatch checkout> log -1` against the fix commit's date
(2026-07-30) to tell whether a given binary needs this patch.

## Evidence

Verified directly in the `tesseracode/tesserapatch` source tree at `d8e3e15`:

- The four new tests in `internal/workflow/verify_committed_range_test.go`
  (`TestRunVerify_CommittedRange_NoParents_V8PassesAgainstRecordedBase`,
  `TestRunVerify_CommittedRange_InvalidPatchStillFails`,
  `TestRunVerify_CommittedRange_UnreachableBaseCommit_FailsExplicitly`,
  `TestRunVerify_CommittedRange_HardParentAlreadyInBase_SkipsReplay`) all
  pass (`go test ./internal/workflow/... -run TestRunVerify_CommittedRange
  -v`).
- The full test suite (`go test ./...`) passes with no regressions.
- `go build ./...` succeeds.
- Per the commit message, a real-world reproduction against a `tpatch`
  feature in a separate worktree flipped `tpatch verify`'s
  `post_apply_patch_replay_clean` check from failed to passed with no
  changes to that feature's own source, only its `status.json` verify
  overlay and the auto-generated `FEATURES.md` dashboard.

## Not in scope

This patch and file only cover the `tpatch` CLI's own verifier bug. They do
not add, modify, or remove anything in this fork's `.tpatch/` workspace
config, `FEATURES.md`, or tracked features — see `.tpatch/README.md` and
`docs/fork/PATCHES.md` for those.
