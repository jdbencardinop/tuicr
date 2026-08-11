---
id: support-gitlab-custom-ports
title: Support self-hosted GitLab custom ports
type: prototype
mode: AFK
status: open
owner:
blocked_by: []
---

## Question

Model a self-hosted GitLab logical hostname, API host/port, and HTTP/HTTPS
scheme separately so full MR URLs with custom ports work without rewriting the
target.

## Completion

Custom-port HTTP and HTTPS MR URLs generate valid `glab` commands while
gitlab.com and standard-port self-hosted behavior remain unchanged. Parser and
command tests plus the disposable GitLab fixture pass.

## Priority

This is not currently a release blocker because a verified portless logical
hostname plus `glab` `api-host`/`api-protocol` configuration works.
[Private continuation issue #2](https://github.com/jdbencardinop/diffreviewtui/issues/2)
tracks the native fix.
