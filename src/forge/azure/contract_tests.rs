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

use std::sync::{Mutex, OnceLock};

use crate::error::TuicrError;
use crate::forge::azure::backend::AzureDevOpsBackend;
use crate::forge::azure::models::AdoThreadStatus;
use crate::forge::azure::test_support::{MockResponse, start_mock_server};
use crate::forge::submit::SubmitEvent;
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeRepository, PullRequestDetails, PullRequestListQuery,
    PullRequestTarget,
};

const PAT_ENV_VAR: &str = "AZURE_DEVOPS_EXT_PAT";
const MOCK_PAT: &str = "s3cr3t-mock-pat-value";

/// Serializes tests that set the process-wide `$AZURE_DEVOPS_EXT_PAT` env
/// var, mirroring `crate::forge::giteafj::backend::tests::token_env_lock`.
fn pat_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Run `body` with `$AZURE_DEVOPS_EXT_PAT` set to [`MOCK_PAT`], restoring
/// whatever value (or absence) was previously there afterward. Holds
/// [`pat_env_lock`] for the whole call so concurrent test threads never
/// race each other's temporary value — every test in this module needs
/// this, since every one exercises the real `client_for`/`resolve_auth`
/// path (unlike Gitea's mock tests, which mostly bypass token resolution
/// by hand-building a client; this module intentionally always goes
/// through the real path since Azure's auth wiring is part of what
/// requirement 3/7 asks to be contract-tested).
fn with_mock_pat<T>(body: impl FnOnce() -> T) -> T {
    let _guard = pat_env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var(PAT_ENV_VAR).ok();
    // SAFETY: guarded by `pat_env_lock` above, so no other test thread
    // observes a partial value while this one mutates the process-wide
    // env var.
    unsafe {
        std::env::set_var(PAT_ENV_VAR, MOCK_PAT);
    }
    let result = body();
    unsafe {
        match previous {
            Some(v) => std::env::set_var(PAT_ENV_VAR, v),
            None => std::env::remove_var(PAT_ENV_VAR),
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

#[test]
fn should_create_review_with_comment_thread_and_approve_vote() {
    with_mock_pat(|| {
        let responses = [
            (
                "POST /contoso/widgets/_apis/git/repositories/api/pullrequests/42/threads?api-version=7.1".to_string(),
                MockResponse::json(200, include_str!("fixtures/thread_create_response.json")),
            ),
            (
                "GET /contoso/_apis/connectionData?api-version=7.1".to_string(),
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
                "GET /contoso/_apis/connectionData?api-version=7.1".to_string(),
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
                "GET /contoso/_apis/connectionData?api-version=7.1".to_string(),
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
            .expect("thread_provider_mapping should succeed")
            .expect("thread has a threadContext, so mapping should be Some");

        assert_eq!(
            mapping["threadContext"]["filePath"],
            "/src/forge/azure/backend.rs"
        );
    });
}
