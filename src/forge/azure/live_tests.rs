//! Live end-to-end checks against a real, disposable Azure DevOps
//! organization/project driven directly through `AzureDevOpsBackend` (not
//! just curl).
//!
//! These are `#[ignore]`d by default — they never run under a plain
//! `cargo test`, and this task never runs them: no live external Azure
//! DevOps organization is approved (see
//! `docs/follow-on-map/tickets/12-implement-azure-adapter.md`'s "Official
//! evidence" constraint and its sibling
//! `04-provision-provider-sandboxes.md`, which is itself open/blocked on
//! sandbox provisioning). This module exists purely as the harness a
//! future, explicitly-approved sandbox run would use — mirroring
//! `crate::forge::giteafj::live_tests`'s same env-var-gated shape — not as
//! evidence this adapter has ever been exercised against a real
//! organization.
//!
//! They only activate when a caller (a shell harness that has already
//! provisioned a disposable Azure DevOps organization/project, with a
//! token-bearing user and an open PR seeded) sets every one of:
//!
//! - `TUICR_LIVE_AZURE_HOST` — e.g. `https://dev.azure.com` (cloud) or an
//!   on-prem Server base URL (see `crate::forge::azure::base_url_from_host`
//!   for the accepted forms).
//! - `TUICR_LIVE_AZURE_OWNER` — the `{organization}/{project}` (or
//!   `{collection}/{project}` for Server) owner path this module's
//!   `ForgeRepository::owner` uses (see `crate::forge::azure` module docs).
//! - `TUICR_LIVE_AZURE_REPO`, `TUICR_LIVE_AZURE_PR`
//! - `TUICR_LIVE_AZURE_PR2` — a second, distinct open PR number in the same
//!   repository, used only to prove real multi-page `list_pull_requests`
//!   pagination against a live instance. Optional: pagination assertions
//!   are skipped (not silently passed) when unset.
//! - `AZURE_DEVOPS_EXT_PAT` (or a working `az login` session) — per
//!   `crate::forge::azure::auth::resolve_auth`'s existing contract.
//!
//! Run with e.g.:
//! ```text
//! TUICR_LIVE_AZURE_HOST=https://dev.azure.com TUICR_LIVE_AZURE_OWNER=contoso/widgets \
//! TUICR_LIVE_AZURE_REPO=api TUICR_LIVE_AZURE_PR=1 AZURE_DEVOPS_EXT_PAT=... \
//! cargo test --lib forge::azure::live_tests -- --ignored --nocapture
//! ```

#![cfg(test)]

use std::path::PathBuf;

use crate::forge::azure::backend::AzureDevOpsBackend;
use crate::forge::submit::{GhSide, InlineComment, SubmitEvent};
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeRepository, PullRequestListQuery, PullRequestTarget,
};

struct LiveEnv {
    host: String,
    owner: String,
    repo: String,
    pr_number: u64,
    pr2_number: Option<u64>,
}

fn live_env() -> Option<LiveEnv> {
    Some(LiveEnv {
        host: std::env::var("TUICR_LIVE_AZURE_HOST").ok()?,
        owner: std::env::var("TUICR_LIVE_AZURE_OWNER").ok()?,
        repo: std::env::var("TUICR_LIVE_AZURE_REPO").ok()?,
        pr_number: std::env::var("TUICR_LIVE_AZURE_PR").ok()?.parse().ok()?,
        pr2_number: std::env::var("TUICR_LIVE_AZURE_PR2")
            .ok()
            .and_then(|v| v.parse().ok()),
    })
}

fn repository(env: &LiveEnv) -> ForgeRepository {
    ForgeRepository::azure_devops(&env.host, &env.owner, &env.repo)
}

/// End-to-end smoke test: list/get PR, commits, review threads/metadata,
/// then post a comment thread and cast an approve vote via
/// `create_review`. Exercises every `ForgeBackend` method this backend
/// implements against a live instance, honoring the same evidenced,
/// explicit gaps this module's unit/mock tests already assert (no diff
/// endpoint without a local checkout, no review-requested list scope, no
/// review-history metadata) rather than asserting behavior this adapter
/// never claimed to support.
#[test]
#[ignore = "requires a live, explicitly-approved disposable Azure DevOps organization; \
            never run for this task (see module docs) — no live sandbox is currently \
            approved (docs/follow-on-map/tickets/04-provision-provider-sandboxes.md)"]
fn should_drive_full_review_lifecycle_against_live_instance() {
    let Some(env) = live_env() else {
        eprintln!("skipping: TUICR_LIVE_AZURE_* env vars not set");
        return;
    };
    let repo = repository(&env);
    let backend = AzureDevOpsBackend::new(Some(repo.clone()));

    let listed = backend
        .list_pull_requests(PullRequestListQuery::first_page(repo.clone(), 20))
        .expect("list_pull_requests should succeed");
    assert!(
        listed
            .pull_requests
            .iter()
            .any(|pr| pr.number == env.pr_number),
        "expected PR #{} in list_pull_requests results",
        env.pr_number
    );

    // Real multi-page pagination evidence via `$skip`/`$top`, only when a
    // second distinct open PR was seeded (a single PR can never exercise a
    // genuine `has_more` transition).
    if let Some(pr2_number) = env.pr2_number {
        let page1 = backend
            .list_pull_requests(PullRequestListQuery::first_page(repo.clone(), 1))
            .expect("list_pull_requests page 1 should succeed");
        assert_eq!(page1.pull_requests.len(), 1);
        assert!(
            page1.has_more,
            "with 2 open PRs and page_size=1, page 1 must report has_more=true"
        );

        let page2_query = PullRequestListQuery {
            repository: repo.clone(),
            already_loaded: page1.total_loaded,
            page_size: 1,
            scope: crate::forge::traits::PullRequestListScope::Open,
        };
        let page2 = backend
            .list_pull_requests(page2_query)
            .expect("list_pull_requests page 2 should succeed");
        assert_eq!(page2.pull_requests.len(), 1);

        let mut seen: Vec<u64> = page1
            .pull_requests
            .iter()
            .chain(page2.pull_requests.iter())
            .map(|pr| pr.number)
            .collect();
        seen.sort_unstable();
        let mut expected = vec![env.pr_number, pr2_number];
        expected.sort_unstable();
        assert_eq!(
            seen, expected,
            "the union of both pages must contain exactly the two known open PRs"
        );
    } else {
        eprintln!(
            "skipping multi-page pagination evidence: TUICR_LIVE_AZURE_PR2 not set \
             (a single PR cannot exercise a has_more transition)"
        );
    }

    let target = PullRequestTarget::with_repository(
        repo.clone(),
        env.pr_number,
        format!("{}/{}/pullrequest/{}", env.owner, env.repo, env.pr_number),
    );
    let pr = backend
        .get_pull_request(target)
        .expect("get_pull_request should succeed");
    assert_eq!(pr.number, env.pr_number);
    assert!(!pr.head_sha.is_empty());
    assert!(!pr.base_sha.is_empty());

    // No documented unified-diff endpoint exists (see this crate's
    // `backend.rs` top-of-file doc comment) — this call is *expected* to
    // fail with `TuicrError::UnsupportedOperation` when no local checkout
    // is configured, which is exactly what this live harness leaves
    // unset. Asserting the typed error here (rather than skipping the
    // call) keeps this evidenced gap honest even in the live harness.
    let diff_result = backend.get_pull_request_diff(&pr);
    assert!(
        matches!(
            diff_result,
            Err(crate::error::TuicrError::UnsupportedOperation(_))
        ),
        "get_pull_request_diff without a local checkout should return a typed \
         UnsupportedOperation, not succeed or panic; got: {diff_result:?}"
    );

    let commits = backend
        .list_pull_request_commits(&pr)
        .expect("list_pull_request_commits should succeed");
    assert!(!commits.is_empty(), "PR should have at least one commit");

    let threads_before = backend
        .list_review_threads(&pr)
        .expect("list_review_threads should succeed");

    let metadata = backend
        .list_pull_request_review_metadata(&pr)
        .expect("list_pull_request_review_metadata should succeed");
    // Evidenced gap: Azure DevOps has no review-history/timeline API, so
    // this always returns the empty default — asserted explicitly here so
    // a future live run can't silently start returning rows without this
    // test being updated to explain why.
    assert!(
        metadata.reviews.is_empty(),
        "list_pull_request_review_metadata should still be the documented empty default"
    );

    let comment = InlineComment {
        path: PathBuf::from("README.md"),
        line: 1,
        side: GhSide::Right,
        counterpart_line: None,
        start_line: None,
        start_side: None,
        old_path: None,
        body: "Live-fixture comment from the Azure DevOps adapter harness.".to_string(),
        comment_id: "live-fixture-azure".to_string(),
    };
    let request = CreateReviewRequest {
        event: SubmitEvent::Approve,
        commit_id: &pr.head_sha,
        body: "",
        comments: &[comment],
    };
    let response = backend
        .create_review(&pr, request)
        .expect("create_review with Approve should succeed");
    assert_eq!(response.state, "Approved");

    let threads_after = backend
        .list_review_threads(&pr)
        .expect("list_review_threads after create_review should succeed");
    assert!(
        threads_after.len() > threads_before.len(),
        "create_review should have posted a new thread"
    );
}
