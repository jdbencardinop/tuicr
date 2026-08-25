---
id: fix-azure-connection-data-version
title: Use the supported Azure Connection Data API version
type: implementation
mode: autonomous
status: closed
owner: copilot
blocked_by: []
---

## Question

Why do live Azure DevOps vote operations fail before reaching the reviewer
endpoint when resolving the authenticated viewer ID?

## Resolution

Azure DevOps rejected stable `api-version=7.1` for Connection Data with
`VssInvalidPreviewVersionException`. Microsoft Learn documents
`7.1-preview.1`. Commit `50c4580` pins only Connection Data to that version;
all Git endpoints remain on stable 7.1. The 60-test focused Azure suite passed,
and the live rerun resolved the viewer ID and reached the vote endpoint.

Tracked feature: `fix-azure-connection-data-version`.
