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
doc comment for the same list).

## Source citations (primary — official Microsoft Learn REST reference)

- Pull Requests - Get:
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get>
- Pull Requests - Get Pull Requests (list):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get-pull-requests>
- Pull Request Iterations - List:
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-iterations/list>
- Pull Request Threads - List / Create / Update:
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-threads>
- Pull Request Thread Comments - Create:
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-thread-comments/create>
- Pull Request Reviewers - Create (vote):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-reviewers/create-pull-request-reviewer>
- Pull Request Commits (continuation-token pagination):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-commits/get-pull-request-commits>
- `IdentityRefWithVote` vote value semantics
  (`10`/`5`/`0`/`-5`/`-10`):
  <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get#identityrefwithvote>

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
| `commits_page1.json` / `commits_page2.json` | `GET .../commits` | `list_pull_request_commits` (continuation-token pagination) |
| `connection_data.json` | `GET _apis/connectionData` | `create_review`/`cast_vote` (viewer identity) |
| `thread_create_response.json` | `POST .../threads` | `create_review` (comment path) |
| `vote_update_response.json` | `PUT .../reviewers/{id}` | `create_review`/`cast_vote` (vote path) |

These are consumed via `include_str!` from
`src/forge/azure/contract_tests.rs`'s mock-HTTP-server tests — never over a
real network connection.
