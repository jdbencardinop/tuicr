# Analysis: fix Azure Connection Data API version

## Observed behavior

The approved 2026-08-25 Azure DevOps live probe successfully created a
thread, posted a reply, resolved it, and reopened it. The first vote operation
then failed before mutation because `viewer_id` requested
`connectionData?api-version=7.1`.

Azure DevOps returned `VssInvalidPreviewVersionException` and required the
preview API suffix. Microsoft Learn documents the 7.1 Connection Data route
as `api-version=7.1-preview.1`.

## Existing seam

`AzureDevOpsBackend::viewer_id` is the sole caller of Connection Data. The
rest of the adapter correctly uses stable Git API version 7.1. Three contract
tests already assert the exact Connection Data request.

## Recommendation

Pin only Connection Data to `7.1-preview.1` and update the exact request
expectations. Do not change the shared stable Git API helper or any other
endpoint.
