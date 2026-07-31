# Azure DevOps fixtures — provenance

Every JSON file in this directory is a **hand-constructed, sanitized**
example (fake organization/project/repository/identity names, fake GUIDs,
fake dates) built to match the *field names, casing, and types* documented
by official Microsoft sources. None of these were captured from a live
Azure DevOps organization — per this task's constraints, no live external
Azure DevOps sandbox is approved, and none was contacted.

No invented fields: every key present in these fixtures also appears in
`src/forge/azure/models.rs`'s corresponding `Ado*` struct, which was itself
built directly from the citations below (see that file's own top-of-module
doc comment for the same list). The one exception is `item_content.txt`,
which is not JSON at all — see the Items - Get citation below for why.

## Source citations (primary — official Microsoft Learn REST reference)

- Pull Requests - Get:
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get>
- Pull Requests - Get Pull Requests (list):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get-pull-requests>
- Pull Request Iterations - List:
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-iterations/list>
- Pull Request Threads - List / Create / Update:
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-threads>
  (the Create page's sample response,
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-threads/create?view=azure-devops-rest-7.1>,
  is the source for `thread_create_response.json`'s
  `rightFileStart`/`rightFileEnd` shape: a non-`1` `offset` and a
  start/end spanning different `line`s, proving `offset` is a real,
  independent column position, not always `1`)
- `CommentThreadContext` schema (same Threads - Create page): defines
  `leftFileStart`/`leftFileEnd`/`rightFileStart`/`rightFileEnd` as four
  independent optional fields — nothing in the schema restricts them to
  only one populated side at a time. `thread_dual_side_response.json`
  populates all four to prove `thread_provider_mapping` preserves a
  genuinely simultaneous left+right anchor verbatim; unlike this file's
  other fixtures, no single official *sample response* shows all four
  populated together (Microsoft's own samples only ever populate one
  side), so this specific combination is schema-legal but hand-built,
  not itself lifted from one official example.
- Pull Request Thread Comments - Create:
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-thread-comments/create>
- Pull Request Reviewers - Create (vote):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-reviewers/create-pull-request-reviewer>
- Pull Request Commits (continuation-token pagination):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-commits/get-pull-request-commits>
- `IdentityRefWithVote` vote value semantics
  (`10`/`5`/`0`/`-5`/`-10`):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get#identityrefwithvote>
- Items - Get (file content, no unified-diff API; used only as the
  no-local-checkout fallback for `fetch_file_lines`/`file_line_count`):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/items/get?view=azure-devops-rest-7.1>
  — confirms `includeContent` only takes effect combined with
  `$format=json`; without it, the endpoint returns the item's raw content
  directly as the response body (not a JSON envelope), which is why
  `item_content.txt` (unlike every other fixture here) is plain text, not
  JSON.
- Rate limits and throughput units (`Retry-After`/`X-RateLimit-*` headers,
  TSTU, `TF400733`):
  <https://learn.microsoft.com/en-us/azure/devops/integrate/concepts/rate-limits?view=azure-devops>
  — source for `client.rs`'s `RETRY_AFTER_HEADER`/
  `RATE_LIMIT_REMAINING_HEADER`/`RATE_LIMIT_LIMIT_HEADER` constants and the
  `AdoResponse.retry_after`/`rate_limit_remaining`/`rate_limit_limit`
  fields; also documents that `require_success` must only *surface* these
  values, never sleep/retry on them automatically (no headers-driven
  fixture file needed — the mock response in
  `contract_tests.rs::should_surface_retry_after_and_rate_limit_headers_without_auto_retrying`
  sets them directly via `MockResponse::with_header`).

## Source citations (secondary — Microsoft's own generated SDK/tooling,
used only for nested shapes Learn's rendered property tables truncate)

- `authenticatedUser.id` shape on Connection Data:
  <https://learn.microsoft.com/en-devops/extend/reference/client/interfaces/connectiondata>
  (Azure DevOps Extension API JS SDK reference — confirms the
  `authenticatedUser` field's existence/shape; the exact top-level REST
  path `GET {org-scope}/_apis/connectionData?api-version=7.1` is
  community/tooling-evidenced, not itself a dedicated Learn REST-reference
  page — see `src/forge/azure/backend.rs`'s `viewer_id` doc comment for
  the same distinction).
- `azure_devops_rust_api` (generated from Microsoft's own published
  OpenAPI/Swagger spec) for a small number of nested field names Learn's
  rendered tables truncate (e.g. `changeEntries[].item`).

## Files

| File | Endpoint | Used by |
| --- | --- | --- |
| `pull_request_get.json` | `GET .../pullrequests/{id}` | `get_pull_request` |
| `pull_request_list.json` | `GET .../pullrequests` | `list_pull_requests` |
| `iterations_list.json` | `GET .../iterations` | `list_review_threads` (staleness) |
| `threads_list.json` | `GET .../threads` | `list_review_threads` |
| `commits_page1.json` / `commits_page2.json` / `commits_page3.json` | `GET .../commits` | `list_pull_request_commits` (continuation-token pagination; `commits_page3.json` backs the wire-level percent-encoding regression test for `x-ms-continuationtoken` values containing reserved query characters) |
| `connection_data.json` | `GET _apis/connectionData` | `create_review`/`cast_vote` (viewer identity) |
| `thread_create_response.json` | `POST .../threads` | `create_review` (comment path) |
| `vote_update_response.json` | `PUT .../reviewers/{id}` | `create_review`/`cast_vote` (vote path) |
| `item_content.txt` | `GET .../items?path=...&versionDescriptor.version=...` | `fetch_file_lines`/`file_line_count` (no-local-checkout fallback) |
| `thread_dual_side_response.json` | `GET .../threads/{id}` | `thread_provider_mapping` (simultaneous left+right anchor preservation) |

These are consumed via `include_str!` from
`src/forge/azure/contract_tests.rs`'s mock-HTTP-server tests — never over a
real network connection.
