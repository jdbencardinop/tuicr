---
id: provision-provider-sandboxes
title: Provision approved provider sandboxes
type: task
mode: HITL
status: closed
owner: copilot
blocked_by: []
---

## Question

Create or approve disposable GitHub, GitLab, and Azure DevOps test targets so
live mutation parity can be proven, alongside the local version-pinned
Gitea/Forgejo instances already in place.

## Completion

Each target has minimum-scope credentials, teardown instructions, no private
source, and explicit permission for automated comment/review writes.

## Status

- **Done:** local, version-pinned **Gitea 1.24** and **Forgejo 16** —
  verified 2026-07-30 against reproducible, disposable Docker fixtures with
  automatic credential/token generation and teardown.
- **Done:** self-hosted **GitLab 19.2.1** — explicitly approved 2026-08-11,
  deployed on the remote VM behind loopback + SSH/Azure Bastion forwarding,
  exercised with disposable one-day credentials and the public fixture, then
  fully torn down.
- **Done:** disposable **GitHub** and **Azure DevOps** targets were explicitly
  approved, exercised with synthetic-only changes, and torn down on
  2026-08-25. Their live implementation gaps are tracked by
  `validate-live-provider-parity`.

### GitLab self-hosted result — 2026-08-11

GitLab is intentionally out of scope for the WSL baseline itself and deferred
to this provider-sandbox ticket. A self-hosted Community Edition target is
feasible:

- The current Ubuntu/WSL host has 16 CPUs, 31 GiB RAM, about 700 GiB free
  storage, a healthy native Linux Docker Engine, Docker Compose, and free
  candidate ports 8929/2224. This exceeds GitLab's documented single-node
  baseline of 8 vCPU, 16 GB RAM, and 40 GB application storage. Running there
  would validate real GitLab API semantics but only loopback networking.
- The designated remote VM is now reachable through an Azure Bastion tunnel
  derived from CDE metadata without hardcoded Azure resource identifiers. It
  has 16 CPUs, about 63 GiB RAM, about 164 GiB free storage, Ubuntu 24.04, a
  healthy native Docker Engine, and free candidate ports 8929/2224. It is the
  preferred target because a loopback-only GitLab container plus the verified
  tunnel gives WSL a genuinely remote service path without public exposure.
- The tunnel lifecycle was exercised end to end: start, listener detection,
  SSH alias configuration, non-interactive SSH, stop with child-process
  cleanup, restart, and idempotent start all passed. Exactly one managed tunnel
  process remains running.
- Tuicr already supports self-hosted GitLab: remotes whose hostname contains
  `gitlab` are recognized, other custom domains can be selected from `glab`
  configuration, `glab api` receives `--hostname`, and MR commands receive a
  full repository URL.
- A deterministic hostname such as
  `gitlab.127.0.0.1.sslip.io:8929` resolves to the tunnel/local listener and
  satisfies Tuicr's host detection without editing `/etc/hosts`.
- The current exact CE image candidate is
  `gitlab/gitlab-ce:19.2.1-ce.0` with multi-platform manifest digest
  `sha256:2777b4a990a05a40947437d3c8e0e03347974d8998663d266064cebaff00ba87`
  (Docker Hub observation on 2026-08-11). Re-resolve and record the digest at
  execution time rather than using `latest`.

Recommended topology: bind GitLab only to remote/local loopback on 8929, use
HTTP Git for the disposable fixture (SSH port optional), connect from WSL
through `ssh -L` when using the remote VM, create minimum-scope disposable
users and tokens inside the container, and tear down only the named
container/volumes created by the run. No runner, registry, private source,
public listener, or mail flow is needed.

The pinned image was deployed and resolved as GitLab 19.2.1. The run created
only two disposable users, one private fixture project, and one merge request.
Credentials were one-day PATs stored only in mode-0600 local state. Live
mutation results are recorded in
`docs/wayfinder/tickets/validate-live-provider-parity.md`.

Teardown removed the exact container, three exact named volumes, local SSH
forward, and local credential/config state. Post-teardown counts were zero
containers and zero matching volumes. The reusable Azure Bastion SSH tunnel
remains running; it contains no GitLab credential or provider data.

The remaining GitHub/Azure sandbox gap keeps the following blocked,
downstream:

- `docs/wayfinder/tickets/validate-live-provider-parity.md` (GitHub/GitLab
  mutation parity and the Azure DevOps adapter's live validation);
- `docs/wayfinder/tickets/release-cross-platform-fork.md` (a release needs
  the above closed first).

## Resolution

All required provider target types have now been provisioned and torn down.
GitHub used a disposable public repository. Azure DevOps used a disposable
draft PR and temporary branch containing only a synthetic file in an approved
directory; it retained zero reviewers and was abandoned before exact branch
deletion. Remaining blockers are adapter/TUI behavior, not target access.
