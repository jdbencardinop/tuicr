//! Live end-to-end checks against a real, disposable Azure DevOps
//! organization/project driven directly through `AzureDevOpsBackend` (not
//! just curl).
//!
//! These are `#[ignore]`d by default — they never run under a plain
//! `cargo test`. A bounded adapter-driven lifecycle was exercised on
//! 2026-08-25 against an explicitly approved disposable draft PR; it found
//! the Connection Data API-version defect fixed by the tracked
//! `fix-azure-connection-data-version` feature and the native-anchor gap
//! tracked by `preserve-azure-native-anchors`. This committed harness still
//! requires an explicitly approved non-draft disposable target because it
//! posts a thread and casts a vote without reversible cleanup.
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
use crate::forge::azure::models::{AdoCommentPosition, AdoCommentThreadContext, AdoThreadStatus};
use crate::forge::submit::{GhSide, InlineComment, SubmitEvent};
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeRepository, PrSessionKey, PullRequestListQuery,
    PullRequestTarget,
};
use crate::model::{ReviewSession, SessionDiffSource};
use crate::review_store::ReviewStore;

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
#[ignore = "requires an explicitly approved non-draft disposable Azure DevOps PR; \
            posts a thread and vote without reversible cleanup"]
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

#[test]
#[ignore = "requires an explicitly approved disposable Azure DevOps draft PR; \
            posts a reversible synthetic thread"]
fn should_preserve_native_anchor_through_durable_import() {
    let Some(env) = live_env() else {
        eprintln!("skipping: TUICR_LIVE_AZURE_* env vars not set");
        return;
    };
    let phase = std::env::var("TUICR_LIVE_AZURE_ANCHOR_PHASE")
        .expect("TUICR_LIVE_AZURE_ANCHOR_PHASE must be seed or verify");
    let marker = std::env::var("TUICR_LIVE_AZURE_ANCHOR_MARKER")
        .expect("TUICR_LIVE_AZURE_ANCHOR_MARKER must identify the synthetic thread");
    let path = std::env::var("TUICR_LIVE_AZURE_ANCHOR_PATH")
        .expect("TUICR_LIVE_AZURE_ANCHOR_PATH must be the approved synthetic file");
    let repo = repository(&env);
    let backend = AzureDevOpsBackend::new(Some(repo.clone()));
    let target = PullRequestTarget::with_repository(
        repo.clone(),
        env.pr_number,
        format!("{}/{}/pullrequest/{}", env.owner, env.repo, env.pr_number),
    );
    let pr = backend
        .get_pull_request(target)
        .expect("get_pull_request should succeed");

    if phase == "seed" {
        let position = AdoCommentPosition {
            line: 60,
            offset: 1,
            extra: Default::default(),
        };
        let thread_id = backend
            .create_thread(
                &pr,
                &marker,
                Some(AdoCommentThreadContext {
                    file_path: path.clone(),
                    left_file_start: None,
                    left_file_end: None,
                    right_file_start: Some(position.clone()),
                    right_file_end: Some(position),
                    extra: Default::default(),
                }),
            )
            .expect("create_thread should succeed");
        backend
            .reply_to_thread(&pr, thread_id, &format!("{marker}:reply"))
            .expect("reply_to_thread should succeed");
        backend
            .update_thread_status(&pr, thread_id, AdoThreadStatus::Fixed)
            .expect("resolving the thread should succeed");
        backend
            .update_thread_status(&pr, thread_id, AdoThreadStatus::Active)
            .expect("reopening the thread should succeed");
        println!("TUICR_LIVE_AZURE_THREAD_ID={thread_id}");
    } else {
        assert!(
            matches!(phase.as_str(), "current" | "verify"),
            "anchor phase must be seed, current, or verify"
        );
    }

    let remote_threads = backend
        .list_review_threads(&pr)
        .expect("list_review_threads should succeed");
    let remote = remote_threads
        .iter()
        .find(|thread| {
            thread
                .comments
                .first()
                .is_some_and(|comment| comment.body == marker)
        })
        .expect("synthetic thread should round-trip through the adapter");
    assert_eq!(
        remote.path.trim_start_matches('/'),
        path.trim_start_matches('/')
    );
    assert!(
        remote.provider_native_anchor.is_some(),
        "provider-native thread context must be retained"
    );
    if phase == "verify" {
        assert!(
            remote.is_outdated,
            "the original iteration anchor must be stale after the head shift"
        );
        assert!(
            remote
                .provider_native_anchor
                .as_ref()
                .and_then(|anchor| anchor.get("pullRequestThreadContext"))
                .is_some(),
            "the shifted thread must retain Azure iteration/tracking context"
        );
    }

    let mut session = ReviewSession::new(
        PathBuf::from("/synthetic/azure-live"),
        pr.base_sha.clone(),
        Some(pr.head_ref_name.clone()),
        SessionDiffSource::PullRequest,
    );
    session.pr_session_key = Some(PrSessionKey::from_details(&pr));
    assert_eq!(
        session.import_remote_review_threads("azure-devops", &remote_threads),
        remote_threads
            .iter()
            .filter(|thread| !thread.comments.is_empty())
            .count()
    );
    let imported_count = session.threads.len();
    assert_eq!(
        session.import_remote_review_threads("azure-devops", &remote_threads),
        0,
        "repeat import must merge by provider ID"
    );
    assert_eq!(
        session.threads.len(),
        imported_count,
        "repeat import must not duplicate durable threads"
    );

    let temp = tempfile::tempdir().expect("temporary ReviewStore root");
    let store = ReviewStore::with_reviews_dir(temp.path().join("reviews"));
    let session_ref = store
        .save_review(&session)
        .expect("saving the durable review should succeed");
    let reloaded = store
        .get_review(&session_ref)
        .expect("reloading the durable review should succeed");
    let persisted = serde_json::to_value(&reloaded).expect("serialize durable review");
    let thread_id = remote.id.as_str();
    let mapping = persisted["threads"]
        .as_array()
        .and_then(|threads| {
            threads.iter().find_map(|thread| {
                let mapping = &thread["provider_mappings"]["azure-devops"];
                (mapping["id"] == thread_id).then_some(mapping)
            })
        })
        .expect("durable provider mapping should exist");
    assert!(
        mapping
            .get("native_anchor")
            .is_some_and(|value| !value.is_null()),
        "durable provider mapping must retain the native anchor"
    );
    println!(
        "TUICR_LIVE_AZURE_ANCHOR_PHASE={phase} thread_id={} outdated={}",
        remote.id, remote.is_outdated
    );
}

#[test]
#[ignore = "requires an explicitly approved disposable non-draft Azure DevOps PR; \
            changes and must restore the current viewer's vote"]
fn should_set_requested_vote_on_live_instance() {
    let Some(env) = live_env() else {
        eprintln!("skipping: TUICR_LIVE_AZURE_* env vars not set");
        return;
    };
    let vote_name = std::env::var("TUICR_LIVE_AZURE_VOTE")
        .expect("TUICR_LIVE_AZURE_VOTE must name the requested vote");
    let vote = match vote_name.as_str() {
        "approved" => crate::forge::azure::models::AdoVote::Approved,
        "approved-with-suggestions" => {
            crate::forge::azure::models::AdoVote::ApprovedWithSuggestions
        }
        "waiting-for-author" => crate::forge::azure::models::AdoVote::WaitingForAuthor,
        "rejected" => crate::forge::azure::models::AdoVote::Rejected,
        "reset" => crate::forge::azure::models::AdoVote::NoVote,
        _ => panic!("unsupported TUICR_LIVE_AZURE_VOTE value: {vote_name}"),
    };
    let repo = repository(&env);
    let backend = AzureDevOpsBackend::new(Some(repo.clone()));
    let target = PullRequestTarget::with_repository(
        repo,
        env.pr_number,
        format!("{}/{}/pullrequest/{}", env.owner, env.repo, env.pr_number),
    );
    let pr = backend
        .get_pull_request(target)
        .expect("get_pull_request should succeed");
    assert!(!pr.is_draft, "vote lifecycle requires a non-draft PR");
    backend
        .cast_vote(&pr, vote)
        .expect("cast_vote should succeed");
    println!(
        "TUICR_LIVE_AZURE_VOTE={vote_name} wire_value={}",
        vote.wire_value()
    );
}
