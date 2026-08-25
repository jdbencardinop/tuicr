//! Mock-HTTP contract tests for [`super::backend::AzureDevOpsBackend`].
//!
//! Every test here spins up `crate::forge::azure::test_support`'s
//! in-process `TcpListener`-based mock server (never a real Azure DevOps
//! organization — none is contacted or approved for this task; see
//! `docs/follow-on-map/tickets/12-implement-azure-adapter.md` and its
//! sibling `04-provision-provider-sandboxes.md`), points a real
//! `AzureDevOpsBackend` at it by setting `ForgeRepository::host` to the
//! mock server's `http://127.0.0.1:<port>` base URL (`base_url_from_host`
//! passes an already-`http(s)://`-prefixed host straight through — see
//! `backend.rs`), and exercises the exact same code path a live call would
//! take: URL/query construction, the real `client_for` → `resolve_auth`
//! auth-header flow, JSON (de)serialization, and pagination/error
//! handling.
//!
//! Fixtures are vendored, sanitized JSON under `fixtures/` — see
//! `fixtures/README.md` for source citations. Nothing here performs a real
//! network call.

#![cfg(test)]

use crate::error::TuicrError;
use crate::forge::azure::backend::AzureDevOpsBackend;
use crate::forge::azure::models::AdoThreadStatus;
use crate::forge::azure::test_support::{MockResponse, env_mutation_lock, start_mock_server};
use crate::forge::submit::SubmitEvent;
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeFileSide, ForgeRepository,
    PullRequestDetails, PullRequestListQuery, PullRequestTarget,
};
use crate::model::diff_types::FileStatus;

const PAT_ENV_VAR: &str = "AZURE_DEVOPS_EXT_PAT";
const MOCK_PAT: &str = "s3cr3t-mock-pat-value";

/// Run `body` with `$AZURE_DEVOPS_EXT_PAT` set to [`MOCK_PAT`] **and**
/// `$PATH` cleared, restoring both afterward. Holds
/// `test_support::env_mutation_lock()` for the whole call so concurrent
/// test threads never race each other's temporary value — every test in
/// this module needs this, since every one exercises the real
/// `client_for`/`resolve_auth` path (unlike Gitea's mock tests, which
/// mostly bypass token resolution by hand-building a client; this module
/// intentionally always goes through the real path since Azure's auth
/// wiring is part of what requirement 3/7 asks to be contract-tested).
///
/// This lock must be the same one `auth.rs`'s own tests use (not a
/// module-local mutex): both modules mutate the same process-wide
/// `$AZURE_DEVOPS_EXT_PAT`/`$PATH` env vars, and Rust's default parallel
/// test harness runs `auth::tests` and this module's tests concurrently
/// on separate threads sharing one process environment — two different
/// mutexes would not serialize access to the same underlying state. This
/// exact race was observed in practice: `auth::tests`'
/// `should_ignore_blank_pat_env_var` (expecting a blank PAT) intermittently
/// observed this module's [`MOCK_PAT`] value mid-check before this was
/// unified onto one shared lock.
///
/// Clearing `$PATH` is load-bearing, not incidental: `resolve_auth` tries
/// the `az` CLI's Entra token first (see `auth.rs`'s doc comment for the
/// audit-mandated precedence), and at least one shared development host
/// used for this ticket has real `az login` state. Without clearing
/// `$PATH`, these "mock-only" contract tests would silently shell out to
/// the real `az account get-access-token` command and mint a genuine
/// bearer token — a live network call this offline-only task forbids.
fn with_mock_pat<T>(body: impl FnOnce() -> T) -> T {
    let _guard = env_mutation_lock()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let previous_pat = std::env::var(PAT_ENV_VAR).ok();
    let previous_path = std::env::var("PATH").ok();
    // SAFETY: guarded by `env_mutation_lock` above, so no other test
    // thread observes a partial value while this one mutates the
    // process-wide env vars.
    unsafe {
        std::env::set_var(PAT_ENV_VAR, MOCK_PAT);
        std::env::set_var("PATH", "");
    }
    let result = body();
    unsafe {
        match previous_pat {
            Some(v) => std::env::set_var(PAT_ENV_VAR, v),
            None => std::env::remove_var(PAT_ENV_VAR),
        }
        match previous_path {
            Some(v) => std::env::set_var("PATH", v),
            None => std::env::remove_var("PATH"),
        }
    }
    result
}

fn repo_at(base_url: &str) -> ForgeRepository {
    ForgeRepository::azure_devops(base_url, "contoso/widgets", "api")
}

/// Parse a captured request body as JSON for structural comparison —
/// `ureq`'s `send_json` pretty-prints (`serde_json::to_vec_pretty`), so
/// comparing against a literal compact-JSON string would be brittle to
/// that formatting choice.
fn body_json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).expect("captured request body should be valid JSON")
}

fn sample_pr_details(repository: ForgeRepository) -> PullRequestDetails {
    PullRequestDetails {
        repository,
        number: 42,
        title: "Add offline Azure DevOps adapter".to_string(),
        url: "https://dev.azure.com/contoso/widgets/_apis/git/repositories/api/pullRequests/42"
            .to_string(),
        state: "active".to_string(),
        is_draft: false,
        author: Some("Jane Reviewer".to_string()),
        head_ref_name: "feature/azure-adapter".to_string(),
        base_ref_name: "main".to_string(),
        head_sha: "1".repeat(40),
        base_sha: "2".repeat(40),
        body: String::new(),
        updated_at: None,
        closed: false,
        merged_at: None,
        diff_start_sha: None,
    }
}

/// Expected `Authorization` header value for [`MOCK_PAT`], per
/// `auth::basic_auth_header`'s documented Basic-auth-with-empty-username
/// scheme.
fn expected_auth_header() -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!(":{MOCK_PAT}"))
    )
}

#[test]
fn should_list_pull_requests_with_paginated_query_and_redacted_auth() {
    with_mock_pat(|| {
        let responses = [(
            "GET /contoso/widgets/_apis/git/repositories/api/pullrequests?api-version=7.1&searchCriteria.status=active&$skip=10&$top=25".to_string(),
            MockResponse::json(200, include_str!("fixtures/pull_request_list.json")),
        )]
        .into_iter()
        .collect();
        let (base_url, requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let query = PullRequestListQuery {
            repository: repo_at(&base_url),
            already_loaded: 10,
            page_size: 25,
            scope: crate::forge::traits::PullRequestListScope::Open,
        };

        let page = backend
            .list_pull_requests(query)
            .expect("list_pull_requests should succeed");

        assert_eq!(page.pull_requests.len(), 1);
        assert_eq!(page.pull_requests[0].number, 42);
        assert_eq!(page.total_loaded, 11);
        // Exactly one request was made (no duplicate/retry), and it carried
        // the real auth header this backend resolved via `client_for`.
        let captured = requests.lock().expect("lock captured requests");
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured[0].headers.get("authorization"),
            Some(&expected_auth_header())
        );
    });
}

#[test]
fn should_get_single_pull_request_details() {
    with_mock_pat(|| {
        let responses = [(
            "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42?api-version=7.1"
                .to_string(),
            MockResponse::json(200, include_str!("fixtures/pull_request_get.json")),
        )]
        .into_iter()
        .collect();
        let (base_url, _requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let repository = repo_at(&base_url);
        let target = PullRequestTarget::with_repository(repository, 42, "42");

        let pr = backend
            .get_pull_request(target)
            .expect("get_pull_request should succeed");

        assert_eq!(pr.number, 42);
        assert_eq!(pr.title, "Add offline Azure DevOps adapter");
        assert_eq!(pr.head_ref_name, "feature/azure-adapter");
        assert_eq!(pr.base_ref_name, "main");
        assert_eq!(pr.head_sha, "1111111111111111111111111111111111111a");
        assert_eq!(pr.base_sha, "2222222222222222222222222222222222222b");
        assert!(!pr.closed);
    });
}

#[test]
fn should_list_review_threads_with_dual_side_anchors_general_thread_and_stale_detection() {
    with_mock_pat(|| {
        let responses = [
            (
                "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42/threads?api-version=7.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/threads_list.json")),
            ),
            (
                "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42/iterations?api-version=7.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/iterations_list.json")),
            ),
        ]
        .into_iter()
        .collect();
        let (base_url, _requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));

        let threads = backend
            .list_review_threads(&pr)
            .expect("list_review_threads should succeed");

        // Thread 104 is `isDeleted: true` and must be filtered out; 101/
        // 102/103 remain.
        assert_eq!(threads.len(), 3);

        let right_anchored = threads.iter().find(|t| t.id == "101").expect("thread 101");
        assert_eq!(right_anchored.path, "src/lib.rs");
        assert_eq!(right_anchored.line, Some(10));
        assert_eq!(
            right_anchored.side,
            crate::forge::remote_comments::RemoteCommentSide::Right
        );
        // Latest iteration is 2; thread 101 compared against iteration 2 —
        // not stale.
        assert!(!right_anchored.is_outdated);

        let left_anchored = threads.iter().find(|t| t.id == "102").expect("thread 102");
        assert_eq!(left_anchored.path, "src/main.rs");
        assert_eq!(
            left_anchored.side,
            crate::forge::remote_comments::RemoteCommentSide::Left
        );
        // Thread 102's `secondComparingIteration` (1) is behind the latest
        // fetched iteration (2) — this is exactly
        // `StaleAnchorSignal::Tracking`'s evidenced staleness signal.
        assert!(left_anchored.is_outdated);

        let general = threads.iter().find(|t| t.id == "103").expect("thread 103");
        assert_eq!(general.path, "");
        assert_eq!(general.line, None);
        assert!(general.is_resolved);
    });
}

#[test]
fn should_paginate_commits_via_continuation_token_with_no_duplicate_requests() {
    with_mock_pat(|| {
        let responses = [
            (
                "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42/commits?api-version=7.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/commits_page1.json"))
                    .with_header("x-ms-continuationtoken", "cont-token-abc"),
            ),
            (
                "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42/commits?api-version=7.1&continuationToken=cont-token-abc".to_string(),
                MockResponse::json(200, include_str!("fixtures/commits_page2.json")),
            ),
        ]
        .into_iter()
        .collect();
        let (base_url, requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));

        let commits = backend
            .list_pull_request_commits(&pr)
            .expect("list_pull_request_commits should succeed");

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].oid, "1111111111111111111111111111111111111a");
        assert_eq!(commits[1].oid, "3333333333333333333333333333333333333c");
        // Exactly two requests total — the continuation loop must stop as
        // soon as a response carries no continuation-token header, never
        // re-requesting a page it already has (no duplicate retries).
        assert_eq!(requests.lock().expect("lock captured requests").len(), 2);
    });
}

/// Regression coverage for a real encoder-mismatch bug found in final
/// review: `fetch_commits` spliced the raw `x-ms-continuationtoken`
/// response header straight into the next page's query string
/// (`extra.push(("continuationToken", token.clone()))`) with no
/// percent-encoding at all — the same class of bug already fixed for
/// `fetch_file_via_api`'s `path`/`versionDescriptor.version` (see that
/// commit), just with a response-derived cursor instead of a
/// caller-supplied file path. Azure DevOps documents this token as
/// "opaque", but opaque does not mean known-safe: it is still spliced
/// into a hand-assembled query string (`with_api_version`), so an
/// unescaped `&`/`=` inside it would inject a bogus extra query
/// parameter or truncate `continuationToken` at the first occurrence —
/// exactly the same query-parameter injection/truncation failure mode.
///
/// Drives three pages through the real `fetch_commits` code path (via
/// the public `list_pull_request_commits` trait method) against a mock
/// server keyed on the exact expected wire-level request target for
/// each page:
/// - page 1 has no `continuationToken` query param and returns a
///   continuation token (`TOKEN_1`) containing `&`, `=`, `+`, `#`, `%`,
///   and a space (kept within visible-ASCII: an HTTP header *value* —
///   which is exactly what this token arrives as, verbatim, over the
///   wire — cannot legally carry raw non-ASCII bytes; `ureq`'s
///   `HeaderValue::to_str()` rejects them outright, so a token
///   containing literal accented characters could never actually arrive
///   this way in practice, and using one here would test an unreachable
///   state instead of the real bug. Unicode-value encoding is already
///   covered, for a caller-supplied value that never round-trips through
///   a response header, by
///   `should_percent_encode_special_characters_in_items_api_query_values`);
/// - page 2's request target must carry `TOKEN_1`'s single, correctly
///   percent-encoded form (asserted verbatim) — a wrong/missing encoder
///   would either send a target the mock never registered (surfacing as
///   a 404) or, worse, silently truncate/inject query parameters. Page 2
///   then returns a **second, distinct** continuation token (`TOKEN_2`)
///   that itself contains a literal `%20` substring (as opposed to an
///   actual space) plus its own `&`/`=`/`+`/`#` characters;
/// - page 3's request target must carry `TOKEN_2`'s own single,
///   correctly percent-encoded form — computed completely
///   independently of `TOKEN_1`/page 2's request. This is what proves
///   there is no double-encoding *or* state carried across loop
///   iterations: `fetch_commits` always re-derives `continuation` from
///   that response's own fresh, raw `x-ms-continuationtoken` header
///   (never from the query string it just sent), so `TOKEN_2`'s literal
///   `%20` must come out as `%2520` (the `%` itself escaped, followed by
///   literal `20`) — if the implementation instead reused/mutated the
///   previously-*sent* (already-escaped) string, or double-encoded
///   `TOKEN_2`, the resulting target would not match this exact
///   expectation and the request would 404 against the mock. Page 3
///   returns no further continuation token, so the loop stops there
///   (exactly 3 requests total, no duplicate retries).
#[test]
fn should_percent_encode_continuation_token_across_paginated_commit_pages() {
    with_mock_pat(|| {
        // Raw (undecoded) tokens, each carrying its own combination of
        // `&`/`=`/`+`/`#`/`%`/space — the reserved characters that must
        // be percent-encoded before ending up in a hand-assembled query
        // string. Kept within visible-ASCII (see the doc comment above
        // for why: this is a response *header* value, which cannot
        // legally carry raw non-ASCII bytes over the wire). Deliberately
        // different per page so page 2's and page 3's request targets
        // can never accidentally collide (the mock server matches by
        // exact `"{METHOD} {target}"` key, so two distinct requests must
        // resolve to two distinct keys). `TOKEN_2` additionally embeds a
        // literal `%20` substring to prove the encoder does not treat
        // already-percent-looking input as pre-encoded (it must still
        // escape the `%` itself, producing `%2520`, not pass `%20`
        // through untouched).
        let token_1 = "tok a&b=c+d#e%z";
        let encoded_token_1 = "tok%20a%26b%3Dc%2Bd%23e%25z";
        let token_2 = "next+tok=1&2#three%20four";
        let encoded_token_2 = "next%2Btok%3D1%262%23three%2520four";

        let page1_target =
            "/contoso/widgets/_apis/git/repositories/api/pullrequests/42/commits?api-version=7.1";
        let page2_target = format!(
            "/contoso/widgets/_apis/git/repositories/api/pullrequests/42/commits?api-version=7.1&continuationToken={encoded_token_1}"
        );
        let page3_target = format!(
            "/contoso/widgets/_apis/git/repositories/api/pullrequests/42/commits?api-version=7.1&continuationToken={encoded_token_2}"
        );

        let responses = [
            (
                format!("GET {page1_target}"),
                MockResponse::json(200, include_str!("fixtures/commits_page1.json"))
                    .with_header("x-ms-continuationtoken", token_1),
            ),
            (
                format!("GET {page2_target}"),
                MockResponse::json(200, include_str!("fixtures/commits_page2.json"))
                    .with_header("x-ms-continuationtoken", token_2),
            ),
            (
                format!("GET {page3_target}"),
                MockResponse::json(200, include_str!("fixtures/commits_page3.json")),
            ),
        ]
        .into_iter()
        .collect();
        let (base_url, requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));

        let commits = backend
            .list_pull_request_commits(&pr)
            .expect("list_pull_request_commits should succeed across all three pages");

        assert_eq!(commits.len(), 3, "should have fetched all three pages");

        let captured = requests.lock().expect("lock captured requests");
        assert_eq!(
            captured.len(),
            3,
            "should make exactly three requests — no duplicate retries"
        );
        assert_eq!(captured[0].target, page1_target, "page 1 request target");
        assert_eq!(
            captured[1].target, page2_target,
            "page 2 request target should carry TOKEN_1's single, correctly percent-encoded form"
        );
        assert_eq!(
            captured[2].target, page3_target,
            "page 3 request target should carry TOKEN_2's own single, correctly \
             percent-encoded form — independent of TOKEN_1/page 2 (no double-encoding or \
             carried-over encoding state across the loop)"
        );

        // Defense-in-depth: an unescaped `&`/`=` in either token would
        // either inflate the query-parameter count (injection) or shift
        // which segment holds which key (truncation) — assert both
        // continuation-page query strings still split into exactly the
        // 2 parameters this endpoint sends on a continuation request,
        // each under its expected key.
        for target in [&page2_target, &page3_target] {
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default();
            let params: Vec<&str> = query.split('&').collect();
            assert_eq!(
                params.len(),
                2,
                "continuation-page query string had an unexpected parameter count: {query}"
            );
            assert!(params[0].starts_with("api-version="));
            assert!(params[1].starts_with("continuationToken="));
        }
    });
}

#[test]
fn should_create_review_with_comment_thread_and_approve_vote() {
    with_mock_pat(|| {
        let responses = [
            (
                "POST /contoso/widgets/_apis/git/repositories/api/pullrequests/42/threads?api-version=7.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/thread_create_response.json")),
            ),
            (
                "GET /contoso/_apis/connectionData?api-version=7.1-preview.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/connection_data.json")),
            ),
            (
                "PUT /contoso/widgets/_apis/git/repositories/api/pullrequests/42/reviewers/eeeeeeee-0000-4000-8000-000000000005?api-version=7.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/vote_update_response.json")),
            ),
        ]
        .into_iter()
        .collect();
        let (base_url, requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));
        let comments = [crate::forge::submit::InlineComment {
            path: "src/forge/azure/backend.rs".into(),
            line: 20,
            side: crate::forge::submit::GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "Consider adding a fixture for the 429 case too.".to_string(),
            comment_id: String::new(),
        }];
        let request = CreateReviewRequest {
            event: SubmitEvent::Approve,
            commit_id: &pr.head_sha,
            body: "",
            comments: &comments,
        };

        let response = backend
            .create_review(&pr, request)
            .expect("create_review should succeed");

        // Vote path wins over the comment-thread id, per `create_review`'s
        // documented triple-population rule.
        assert_eq!(response.state, "Approved");

        let captured = requests.lock().expect("lock captured requests");
        assert_eq!(captured.len(), 3);
        let thread_post = captured
            .iter()
            .find(|r| r.method == "POST")
            .expect("thread creation POST");
        assert!(thread_post.body.contains("Consider adding a fixture"));
        assert!(thread_post.body.contains("\"rightFileStart\""));
        let vote_put = captured
            .iter()
            .find(|r| r.method == "PUT")
            .expect("vote PUT");
        assert_eq!(body_json(&vote_put.body), serde_json::json!({"vote": 10}));
    });
}

#[test]
fn should_map_request_changes_event_to_rejected_vote() {
    with_mock_pat(|| {
        let responses = [
            (
                "GET /contoso/_apis/connectionData?api-version=7.1-preview.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/connection_data.json")),
            ),
            (
                "PUT /contoso/widgets/_apis/git/repositories/api/pullrequests/42/reviewers/eeeeeeee-0000-4000-8000-000000000005?api-version=7.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/vote_update_response.json")),
            ),
        ]
        .into_iter()
        .collect();
        let (base_url, requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));
        let request = CreateReviewRequest {
            event: SubmitEvent::RequestChanges,
            commit_id: &pr.head_sha,
            body: "",
            comments: &[],
        };

        backend
            .create_review(&pr, request)
            .expect("create_review should succeed");

        let vote_put = {
            let captured = requests.lock().expect("lock captured requests");
            captured
                .iter()
                .find(|r| r.method == "PUT")
                .expect("vote PUT")
                .body
                .clone()
        };
        assert_eq!(body_json(&vote_put), serde_json::json!({"vote": -10}));
    });
}

/// Covers audit requirement 7: thread creation must be independently
/// callable, not exist only as an inline step of `create_review`'s
/// bundled comment(s)-then-vote flow. This exercises `create_thread`
/// directly — no vote, no `create_review` call at all — for both an
/// anchored (inline file) and a general (whole-PR) thread.
#[test]
fn should_create_thread_as_a_standalone_operation_independent_of_create_review() {
    with_mock_pat(|| {
        let responses = [(
            "POST /contoso/widgets/_apis/git/repositories/api/pullrequests/42/threads?api-version=7.1".to_string(),
            MockResponse::json(200, include_str!("fixtures/thread_create_response.json")),
        )]
        .into_iter()
        .collect();
        let (base_url, requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));

        let thread_context = crate::forge::azure::models::AdoCommentThreadContext {
            file_path: "/src/forge/azure/backend.rs".to_string(),
            left_file_start: None,
            left_file_end: None,
            right_file_start: Some(crate::forge::azure::models::AdoCommentPosition {
                line: 20,
                offset: 5,
            }),
            right_file_end: Some(crate::forge::azure::models::AdoCommentPosition {
                line: 22,
                offset: 13,
            }),
        };

        let thread_id = backend
            .create_thread(
                &pr,
                "Consider adding a fixture for the 429 case too.",
                Some(thread_context),
            )
            .expect("create_thread should succeed independently of create_review");
        assert_eq!(thread_id, 201);

        let captured = requests.lock().expect("lock captured requests");
        assert_eq!(
            captured.len(),
            1,
            "create_thread must issue exactly one POST — no vote, no bundled create_review call"
        );
        let post = &captured[0];
        assert_eq!(post.method, "POST");
        assert!(post.body.contains("Consider adding a fixture"));
        assert!(post.body.contains("\"rightFileStart\""));
    });
}

#[test]
fn should_reply_to_thread_update_status_and_cast_vote() {
    with_mock_pat(|| {
        let responses = [
            (
                "POST /contoso/widgets/_apis/git/repositories/api/pullrequests/42/threads/101/comments?api-version=7.1".to_string(),
                MockResponse::json(200, r#"{"id": 5, "parentCommentId": 1, "content": "Thanks, fixed."}"#),
            ),
            (
                "PATCH /contoso/widgets/_apis/git/repositories/api/pullrequests/42/threads/101?api-version=7.1".to_string(),
                MockResponse::json(200, r#"{"id": 101, "status": "fixed", "comments": []}"#),
            ),
            (
                "GET /contoso/_apis/connectionData?api-version=7.1-preview.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/connection_data.json")),
            ),
            (
                "PUT /contoso/widgets/_apis/git/repositories/api/pullrequests/42/reviewers/eeeeeeee-0000-4000-8000-000000000005?api-version=7.1".to_string(),
                MockResponse::json(200, r#"{"id": "eeeeeeee-0000-4000-8000-000000000005", "vote": -5}"#),
            ),
        ]
        .into_iter()
        .collect();
        let (base_url, requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));

        backend
            .reply_to_thread(&pr, 101, "Thanks, fixed.")
            .expect("reply should succeed");
        backend
            .update_thread_status(&pr, 101, AdoThreadStatus::Fixed)
            .expect("status update should succeed");
        backend
            .cast_vote(&pr, crate::forge::azure::models::AdoVote::WaitingForAuthor)
            .expect("cast_vote should succeed");

        let captured = requests.lock().expect("lock captured requests");
        assert_eq!(captured.len(), 4);
        let reply = captured
            .iter()
            .find(|r| r.method == "POST")
            .expect("reply POST");
        assert_eq!(
            body_json(&reply.body),
            serde_json::json!({"content": "Thanks, fixed."})
        );
        let status_patch = captured
            .iter()
            .find(|r| r.method == "PATCH")
            .expect("status PATCH");
        assert_eq!(
            body_json(&status_patch.body),
            serde_json::json!({"status": "fixed"})
        );
        let vote_put = captured
            .iter()
            .find(|r| r.method == "PUT")
            .expect("vote PUT");
        assert_eq!(body_json(&vote_put.body), serde_json::json!({"vote": -5}));
    });
}

#[test]
fn should_surface_error_statuses_without_leaking_the_auth_header_or_retrying() {
    with_mock_pat(|| {
        for status in [401, 403, 409, 429, 500, 503] {
            let responses = [(
                "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42?api-version=7.1"
                    .to_string(),
                MockResponse::json(
                    status,
                    format!("{{\"message\": \"mock {status} failure\"}}"),
                ),
            )]
            .into_iter()
            .collect();
            let (base_url, requests) = start_mock_server(responses);
            let backend = AzureDevOpsBackend::new(None);
            let repository = repo_at(&base_url);
            let target = PullRequestTarget::with_repository(repository, 42, "42");

            let err = backend
                .get_pull_request(target)
                .expect_err(&format!("status {status} should surface as an error"));
            let message = format!("{err}");

            assert!(matches!(err, TuicrError::Forge(_)));
            assert!(message.contains(&status.to_string()));
            assert!(
                !message.contains(MOCK_PAT),
                "error message must never leak the auth token: {message}"
            );
            // Exactly one request — a failing status must never trigger an
            // automatic client-side retry.
            assert_eq!(
                requests.lock().expect("lock captured requests").len(),
                1,
                "status {status} should not have been retried"
            );
        }
    });
}

#[test]
fn should_surface_retry_after_and_rate_limit_headers_without_auto_retrying() {
    // Per https://learn.microsoft.com/en-us/azure/devops/integrate/concepts/rate-limits?view=azure-devops
    // ("Best practices"): honor `Retry-After` and monitor `X-RateLimit-*`
    // headers on a throttled response. This proves the client surfaces
    // both in the resulting error (for the caller/human to act on) while
    // still making exactly one request — audit requirement 8 explicitly
    // forbids any automatic retry without ID reconciliation.
    with_mock_pat(|| {
        let responses = [(
            "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42?api-version=7.1"
                .to_string(),
            MockResponse::json(429, "{\"message\": \"TF400733: request blocked\"}")
                .with_header("Retry-After", "30")
                .with_header("X-RateLimit-Remaining", "0")
                .with_header("X-RateLimit-Limit", "200"),
        )]
        .into_iter()
        .collect();
        let (base_url, requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let repository = repo_at(&base_url);
        let target = PullRequestTarget::with_repository(repository, 42, "42");

        let err = backend
            .get_pull_request(target)
            .expect_err("429 should surface as an error");
        let message = format!("{err}");

        assert!(message.contains("retry-after: 30s"), "message: {message}");
        assert!(message.contains("rate-limit: 0/200"), "message: {message}");
        assert!(
            !message.contains(MOCK_PAT),
            "error message must never leak the auth token: {message}"
        );
        assert_eq!(
            requests.lock().expect("lock captured requests").len(),
            1,
            "a Retry-After header must not itself trigger an automatic retry"
        );
    });
}

#[test]
fn should_return_thread_provider_mapping_for_durable_persistence() {
    with_mock_pat(|| {
        let responses = [(
            "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42/threads/101?api-version=7.1".to_string(),
            MockResponse::json(200, include_str!("fixtures/thread_create_response.json")),
        )]
        .into_iter()
        .collect();
        let (base_url, _requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));

        let mapping = backend
            .thread_provider_mapping(&pr, 101)
            .expect("thread_provider_mapping should succeed");

        assert_eq!(
            mapping["threadContext"]["filePath"],
            "/src/forge/azure/backend.rs"
        );
        // Audit requirement 5: the durable mapping must preserve the full
        // line **and** offset for the anchor range — not just the start
        // line `list_review_threads`' `RemoteReviewThread.line` alone
        // exposes. Per the official "Pull Request Threads - Create"
        // sample (`fixtures/README.md`'s citation), `offset` is a
        // meaningful column position within the line, independent of
        // `line` itself, and start/end can span multiple lines.
        assert_eq!(mapping["threadContext"]["rightFileStart"]["line"], 20);
        assert_eq!(mapping["threadContext"]["rightFileStart"]["offset"], 5);
        assert_eq!(mapping["threadContext"]["rightFileEnd"]["line"], 22);
        assert_eq!(mapping["threadContext"]["rightFileEnd"]["offset"], 13);
        // Audit requirement 6: the native `CommentThreadStatus` value must
        // be preserved verbatim in the durable mapping, not just the
        // anchor geometry — `list_review_threads`' `is_resolved` bool
        // alone cannot round-trip Azure's 7-value status enum.
        assert_eq!(mapping["status"], "active");
    });
}

/// Covers audit requirement 5's "dual-side" half directly: Azure's
/// `CommentThreadContext` schema (Microsoft Learn) defines
/// `leftFileStart`/`leftFileEnd`/`rightFileStart`/`rightFileEnd` as four
/// fully independent optional fields — nothing in the schema prevents
/// both the left (base) and right (head) side being populated on the same
/// thread at once (`SideSupport::simultaneous_both_sides()`/
/// `RangeSupport::DualSideOffsets` in `crate::forge::capabilities`).
/// `list_review_threads`' `RemoteReviewThread` (one shared `line`/`side`
/// pair, per the shared `ForgeBackend` trait) necessarily picks a single
/// side, but `thread_provider_mapping`'s durable mapping must not: this
/// proves all four positions (and their `offset`s) survive verbatim.
#[test]
fn should_preserve_both_sides_when_a_thread_context_has_simultaneous_left_and_right_anchors() {
    with_mock_pat(|| {
        let responses = [(
            "GET /contoso/widgets/_apis/git/repositories/api/pullrequests/42/threads/105?api-version=7.1".to_string(),
            MockResponse::json(200, include_str!("fixtures/thread_dual_side_response.json")),
        )]
        .into_iter()
        .collect();
        let (base_url, _requests) = start_mock_server(responses);
        let backend = AzureDevOpsBackend::new(None);
        let pr = sample_pr_details(repo_at(&base_url));

        let mapping = backend
            .thread_provider_mapping(&pr, 105)
            .expect("thread_provider_mapping should succeed");

        let ctx = &mapping["threadContext"];
        assert_eq!(ctx["leftFileStart"]["line"], 8);
        assert_eq!(ctx["leftFileStart"]["offset"], 3);
        assert_eq!(ctx["leftFileEnd"]["line"], 8);
        assert_eq!(ctx["leftFileEnd"]["offset"], 20);
        assert_eq!(ctx["rightFileStart"]["line"], 20);
        assert_eq!(ctx["rightFileStart"]["offset"], 5);
        assert_eq!(ctx["rightFileEnd"]["line"], 22);
        assert_eq!(ctx["rightFileEnd"]["offset"], 13);
        assert_eq!(mapping["pullRequestThreadContext"]["changeTrackingId"], 9);
    });
}

/// Covers the audit's constraint #4 gap: `fetch_file_via_api` (backing
/// `fetch_file_lines`/`file_line_count`'s no-local-checkout fallback) had
/// no mock-contract coverage. Per official docs (Items - Get,
/// <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/items/get?view=azure-devops-rest-7.1>),
/// without `$format=json` the endpoint returns the raw item content
/// directly as the response body — `fixtures/item_content.txt` models
/// that shape (a plain-text file body, not a JSON envelope), matching how
/// `fetch_file_via_api` consumes `response.body` as-is.
#[test]
fn should_fetch_file_content_via_items_api_when_no_local_checkout() {
    with_mock_pat(|| {
        let request = |base_url: &str| ForgeFileLinesRequest {
            repository: repo_at(base_url),
            base_sha: "2222222222222222222222222222222222222b".to_string(),
            head_sha: "1111111111111111111111111111111111111a".to_string(),
            path: "src/lib.rs".into(),
            status: FileStatus::Modified,
            side: ForgeFileSide::Head,
            start_line: 1,
            end_line: 3,
        };
        let items_key = "GET /contoso/widgets/_apis/git/repositories/api/items?api-version=7.1&path=src/lib.rs&versionDescriptor.version=1111111111111111111111111111111111111a&versionDescriptor.versionType=commit&includeContent=true".to_string();

        // Each call below hits the same Items-API endpoint once, so each
        // gets its own single-response mock server (`start_mock_server`
        // shuts down after serving exactly `responses.len()` requests).
        {
            let responses = [(
                items_key.clone(),
                MockResponse::json(200, include_str!("fixtures/item_content.txt")),
            )]
            .into_iter()
            .collect();
            let (base_url, requests) = start_mock_server(responses);
            let backend = AzureDevOpsBackend::new(None);

            let count = backend
                .file_line_count(request(&base_url))
                .expect("file_line_count should succeed via the Items API");
            assert_eq!(count, 3);
            assert_eq!(requests.lock().expect("lock captured requests").len(), 1);
        }
        {
            let responses = [(
                items_key,
                MockResponse::json(200, include_str!("fixtures/item_content.txt")),
            )]
            .into_iter()
            .collect();
            let (base_url, requests) = start_mock_server(responses);
            // No `.with_local_checkout(...)`: forces the Items-API
            // fallback path this test targets, never a local
            // `git show`/`git diff`.
            let backend = AzureDevOpsBackend::new(None);

            let lines = backend
                .fetch_file_lines(request(&base_url))
                .expect("fetch_file_lines should succeed via the Items API");
            assert_eq!(lines.len(), 3);
            assert_eq!(lines[0].content, "fn widget_total(count: u32) -> u32 {");
            assert_eq!(requests.lock().expect("lock captured requests").len(), 1);
        }
    });
}

/// Regression coverage for a real encoder-mismatch bug found in
/// acceptance review: `fetch_file_via_api`'s `path` (and
/// `versionDescriptor.version`) are QUERY-string *values* — the third
/// argument to `with_api_version`, which hand-assembles
/// `key=value&key=value...` with `format!` (see that function's doc
/// comment in `backend.rs`) — not URL PATH segments. The fix now runs
/// them through `encode_query_value` instead of
/// `encode_path_segments`/`encode_path_segment`.
///
/// The path-segment encoder's escape set has no reason to escape
/// `&`/`=`/`+` (they are ordinary characters within one path segment),
/// so a file path containing any of them would previously have been
/// spliced into the query string unescaped — corrupting the request:
/// - an unescaped `&` starts a bogus new query parameter and truncates
///   `path` at that point (query-parameter injection/truncation);
/// - an unescaped `=` inside the value confuses `key=value` parsing.
///
/// Every case below drives the real `fetch_file_via_api` code path
/// (via the public `file_line_count`/`fetch_file_lines` trait methods,
/// exactly as `should_fetch_file_content_via_items_api_when_no_local_checkout`
/// above does) against a mock server keyed on the **exact** expected
/// wire-level request target — `test_support`'s mock server matches
/// `"{METHOD} {target}"` verbatim, so a wrong encoding sends the request
/// to a target the mock never registered and the call fails with a 404
/// ("no mock response registered"), not a false-positive pass. Each case
/// additionally splits the captured target's query string on `&` and
/// asserts exactly 5 parameters survive with their expected keys, so a
/// regression that reintroduces an unescaped `&`/`=` in `path` is caught
/// even if the full-string assertion below it were ever loosened.
#[test]
fn should_percent_encode_special_characters_in_items_api_query_values() {
    with_mock_pat(|| {
        // (test label, raw file path, expected percent-encoded query value)
        let cases: &[(&str, &str, &str)] = &[
            ("ampersand", "src/A&B.rs", "src/A%26B.rs"),
            ("equals", "src/eq=uals.rs", "src/eq%3Duals.rs"),
            ("plus", "src/plus+file.rs", "src/plus%2Bfile.rs"),
            ("hash", "src/hash#file.rs", "src/hash%23file.rs"),
            // `?` has no special meaning once already inside the query
            // component (RFC 3986/WHATWG URL), so it is legitimately
            // left unescaped by `encode_query_value` — asserted here so
            // any future over-escaping regression is caught too.
            ("question", "src/question?mark.rs", "src/question?mark.rs"),
            ("percent", "src/percent%file.rs", "src/percent%25file.rs"),
            ("space", "src/space file.rs", "src/space%20file.rs"),
            (
                "slash_nested",
                "src/nested/dir/file.rs",
                "src/nested/dir/file.rs",
            ),
            (
                "unicode",
                "src/ünïcödé/文件.rs",
                "src/%C3%BCn%C3%AFc%C3%B6d%C3%A9/%E6%96%87%E4%BB%B6.rs",
            ),
        ];

        for (label, raw_path, expected_encoded_path) in cases {
            let expected_target = format!(
                "/contoso/widgets/_apis/git/repositories/api/items?api-version=7.1&path={expected_encoded_path}&versionDescriptor.version=1111111111111111111111111111111111111a&versionDescriptor.versionType=commit&includeContent=true"
            );
            let key = format!("GET {expected_target}");
            let responses = [(
                key,
                MockResponse::json(200, include_str!("fixtures/item_content.txt")),
            )]
            .into_iter()
            .collect();
            let (base_url, requests) = start_mock_server(responses);
            let request = ForgeFileLinesRequest {
                repository: repo_at(&base_url),
                base_sha: "2222222222222222222222222222222222222b".to_string(),
                head_sha: "1111111111111111111111111111111111111a".to_string(),
                path: (*raw_path).into(),
                status: FileStatus::Modified,
                side: ForgeFileSide::Head,
                start_line: 1,
                end_line: 3,
            };
            let backend = AzureDevOpsBackend::new(None);

            let count = backend.file_line_count(request).unwrap_or_else(|err| {
                panic!(
                    "case {label:?} (path {raw_path:?}) should reach the exact expected \
                     mock-registered target without query injection/truncation: {err}"
                )
            });
            assert_eq!(count, 3, "case {label:?} unexpected line count");

            let captured = requests.lock().expect("lock captured requests");
            assert_eq!(
                captured.len(),
                1,
                "case {label:?} should make exactly one request"
            );
            let target = &captured[0].target;
            assert_eq!(
                *target, expected_target,
                "case {label:?} produced an unexpected wire-level request target"
            );

            // Defense-in-depth: an unescaped `&`/`=` inside `path` would
            // either inflate this count (query injection) or shift which
            // segment holds which key (truncation) — assert the query
            // string still splits into exactly the 5 parameters this
            // endpoint sends, each under its expected key.
            let query = target.split_once('?').map(|(_, q)| q).unwrap_or_default();
            let params: Vec<&str> = query.split('&').collect();
            assert_eq!(
                params.len(),
                5,
                "case {label:?} query string had an unexpected parameter count: {query}"
            );
            let expected_keys = [
                "api-version",
                "path",
                "versionDescriptor.version",
                "versionDescriptor.versionType",
                "includeContent",
            ];
            for (param, expected_key) in params.iter().zip(expected_keys) {
                let actual_key = param.split_once('=').map(|(k, _)| k).unwrap_or(param);
                assert_eq!(
                    actual_key, expected_key,
                    "case {label:?} query parameter order/keys shifted: {query}"
                );
            }
        }
    });
}
