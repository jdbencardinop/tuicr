# Specification: capture provider-neutral context for local anchors

## Goal

Keep local line and range comments attached to their uniquely identifiable
content after a diff/head shift while preserving explicit stale or ambiguous
state when relocation is unsafe.

## Acceptance criteria

1. A line or range comment newly created in the TUI receives bounded
   `AnchorContext` from the exact displayed old/new diff side.
2. Review-level and file-level anchors remain context-free and current.
3. Cold-loaded legacy comments are migrated deterministically as before and
   are not retroactively assigned context from a possibly shifted numeric
   position.
4. Ordinary local `:e` reload refreshes context-bearing durable anchors using
   updated content:
   - one exact context match relocates the line or complete range;
   - zero matches yields `Stale`;
   - multiple matches yields `Ambiguous`;
   - no nearest-line or endpoint guessing occurs.
5. A uniquely relocated current durable anchor updates its mirrored legacy
   comment location so unified and side-by-side rendering, annotations,
   navigation, persistence, and export agree on the new row.
6. A relocated range preserves its inclusive span and updates both the legacy
   `line_range` and terminal-line map key.
7. Stale or ambiguous legacy shadows remain at their last known row and show
   the existing durable-thread status suffix.
8. Old- and new-side comments never borrow context from the opposite side.
9. Repeated migration/reload is idempotent: no duplicate threads or comments
   are created, and a stable uniquely matched anchor does not drift.
10. Focused regression tests cover line 42 to 47 after five inserted lines,
    range relocation, missing context, duplicate context, old/new side,
    unchanged file anchors, legacy-shadow placement, and repeated refresh.
11. Formatting, focused tests, clippy with warnings denied, the supported
    locked library suite, `tpatch test`, and `tpatch verify` pass.
12. A real WSL TUI fixture rerun confirms the visible local comment moves or
    receives the correct typed stale/ambiguous label.

## Implementation plan

1. Add side-aware bounded context extraction over `DiffFile` and a validated
   API for attaching it to a context-free line/range anchor.
2. Change App mirroring to identify only newly migrated threads and attach
   context from the current displayed diff.
3. Generalize durable-anchor refresh so ordinary local reload can supply
   updated content as well as PR head-advance paths.
4. Add a `ReviewSession` synchronization helper that re-keys only current
   mirrored line/range comments from their canonical durable targets.
5. Wire refresh and synchronization into `reload_diff_files` before
   annotations are rebuilt.
6. Add focused domain/App/render regressions, run repository gates, perform
   the WSL TUI fixture rerun, and record/verify/land the tracked patch.

## Non-goals

- Retrofitting trustworthy context onto old persisted comments.
- Provider-native remapping or publication.
- GitLab, GitHub, or Azure provider calls.
- Implementing optional issues #2 or #3.
- Guessing a nearby line when exact context is unavailable.

## Accepted tradeoff

When a source cannot provide trustworthy updated content, the anchor is left
at its last known state rather than being declared current. This favors
honesty and compatibility over speculative movement.
