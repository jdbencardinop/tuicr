# Tessera Patch workspace

This `.tpatch/` workspace was created with `tpatch init` (v0.11.3) to prepare
tracked infrastructure for fork customizations. It started empty; new
patch-bearing work now lives under `.tpatch/features/` and is summarized in
`FEATURES.md`.

## Historical fork changes predate this workspace

Every fork customization made before this workspace was created (see the
offline candidate at commit `ca319dc` and its ancestors) was recorded as a
plain Git commit carrying a `Tpatch-Feature: <name>` trailer, e.g.:

```
git log --all --grep="Tpatch-Feature"
```

or, per commit:

```
git log --format='%H %s%n%(trailers:key=Tpatch-Feature)'
```

The authoritative index of that historical work is
[`docs/fork/PATCHES.md`](../docs/fork/PATCHES.md) in this fork repository,
plus the commit trailers themselves. **Do not invent a
historical `.tpatch/features/<slug>/` directory** for that work — no such
tracked feature artifacts (request/plan/exploration/recipe) were produced at
the time, and fabricating them now would misrepresent what was actually
analyzed, defined, explored, and recorded. The current `FEATURES.md` contains
only customizations created after this workspace bootstrap; it is not a
backfill of prior history.

## Workflow for new patch-bearing customizations

Every *new* customization that changes fork source (not documentation-only
research) goes through the full tracked lifecycle, starting from this clean
workspace:

1. `tpatch add <description>` — create the tracked feature request.
2. `tpatch analyze <slug>` → `tpatch define <slug>` → `tpatch explore <slug>`
   — run manually (or via a configured provider) in that order; each phase
   advances feature state and produces the corresponding artifact
   (analysis/plan/exploration) under `.tpatch/features/<slug>/`.
3. Implementation — `tpatch implement <slug>` to generate the deterministic
   apply recipe, then `tpatch apply <slug>` to execute it (or an
   agent-authored equivalent under `--manual`, per the installed skills).
4. `tpatch record <slug>` — capture the resulting patch (tracked + untracked
   files) and `tpatch test <slug>` — run the configured test command and
   record the result.
5. `tpatch verify <slug>` — integrity-check the recipe/dependencies, then
   `tpatch reconcile` — reconcile the feature against upstream once it lands.

A decision, ticket, or research finding is **not** a `tpatch` feature until it
is patch-bearing (i.e. it reaches step 1 above with real source changes in
view). Tracking a decision in `tws decide`, a Wayfinder ticket, or
`docs/fork/PATCHES.md` does not by itself create or require a `.tpatch/`
feature entry.

## Committed-range verification caveat

Before running `tpatch record`/`tpatch verify` against a **committed range**
capture (`--auto`, `--from`/`--to`, or `--commit-range`), confirm the `tpatch`
binary in use either:

- is built from a version containing upstream commit `d8e3e15` (fix: root the
  V7/V8 replay shadow at the recorded base commit for committed-range
  captures, not live HEAD), or
- has the equivalent locally-fixed committed-range verifier behavior applied.

A `tpatch` binary built from v0.11.3 (release commit `84a2f88`) predates
`d8e3e15` (authored 2026-07-30, one commit later) and will double-apply the
target's own commits when verifying a committed-range capture, causing
spurious `git apply --check` failures. Rebuild or update `tpatch` before
recording any committed-range capture; working-tree (`tpatch record` without
a range) capture is unaffected. See
[`../docs/fork/TPATCH.md`](../docs/fork/TPATCH.md) for a vendored patch and
build instructions to apply this fix locally until an upstream `tpatch`
release contains it.

## Provider configuration is per-machine

`.tpatch/config.yaml` ships with `provider.base_url`/`provider.model` left
blank. `tpatch init` auto-detects a local provider (e.g. a localhost
`copilot-api` proxy) on the machine that runs it, but that endpoint is
developer-local and must not be committed as a shared default — configure it
on each machine with `tpatch provider set --preset <preset> --base-url
<url> --model <model>`. By default this writes the **global** config (outside
this repo); add `--repo` to instead write to this repo's
`.tpatch/config.yaml` (not recommended, since it would reintroduce a
machine-local endpoint into tracked config). Use the separate global
`--path <dir>` flag only to point any `tpatch` command at a different
repository/worktree than the current directory — it does not control
where provider config is stored.

## Generated skill assets and what is tracked

`tpatch init` also generates per-tool skill/instruction files. This repo's
upstream `.gitignore` already excludes `.claude/` project-wide (added upstream
in `04a44a6`, predating this fork), so `.claude/skills/tessera-patch/SKILL.md`
is intentionally **not** tracked here — that follows the existing convention
rather than overriding it. The following generated assets have no such
upstream exclusion and are tracked as plain, portable instructions (no
secrets, machine-specific paths, or provider endpoints):

- `.github/skills/tessera-patch/SKILL.md` (Copilot)
- `.github/prompts/tessera-patch-apply.prompt.md` (Copilot prompt)
- `.cursor/rules/tessera-patch.mdc` (Cursor)
- `.windsurfrules` (Windsurf)
- `.tpatch/workflows/tessera-patch-generic.md` (generic/other agents)

None of these edit `AGENTS.md` or `CLAUDE.md`; they are additive files that
describe the `tpatch` methodology and defer to this repo's own AGENTS.md for
project-specific rules.
