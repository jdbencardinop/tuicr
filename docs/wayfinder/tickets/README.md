# Fork Wayfinder tickets

Each file is one open question or prerequisite that still gates a real
release of this fork. Frontmatter fields:

- `id`: stable local name;
- `title`: human-readable identity;
- `type`: `research`, `prototype`, `grilling`, or `task`;
- `mode`: `AFK` or `HITL`;
- `status`: `open`, `in-progress`, `blocked`, or `closed`;
- `owner`: active claim, empty when unclaimed;
- `blocked_by`: ticket names that must close first.

The frontier is the set of open, unclaimed tickets whose blockers are closed.
Detailed resolutions live in the ticket. `docs/wayfinder/map.md` contains only
linked gists.

This directory intentionally holds only the tickets that are still
open/blocking the fork's release/upstream frontier. Closed research tickets
(candidate verification, provider semantics, macOS pilots, the recommendation
itself, etc.) are not copied here — see the read-only research workspace's
`docs/wayfinder/tickets/` and `docs/follow-on-map/tickets/` for the full,
closed history behind each accepted decision in `docs/fork/DECISIONS.md`.
