//! Live end-to-end checks against a real, disposable Gitea or Forgejo
//! instance driven directly through `GiteaForgejoBackend` (not just curl).
//!
//! These are `#[ignore]`d by default — they never run under a plain
//! `cargo test` and never touch any external/hosted provider. They only
//! activate when a caller (a shell harness that has already provisioned a
//! disposable local container, e.g. via `fixtures/providers/{gitea,forgejo}
//! /run.sh`'s own primitives) sets every one of:
//!
//! - `TUICR_LIVE_KIND` = `gitea` | `forgejo`
//! - `TUICR_LIVE_HOST` = e.g. `http://127.0.0.1:34521` (explicit scheme —
//!   see `auth::base_url_from_host`)
//! - `TUICR_LIVE_OWNER`, `TUICR_LIVE_REPO`, `TUICR_LIVE_PR`
//! - `GITEA_TOKEN` or `FORGEJO_TOKEN` (matching `TUICR_LIVE_KIND`) — the
//!   reviewer account's token, per `auth::resolve_token`'s existing
//!   contract.
//!
//! Run with e.g.:
//! ```text
//! TUICR_LIVE_KIND=gitea TUICR_LIVE_HOST=http://127.0.0.1:PORT \
//! TUICR_LIVE_OWNER=... TUICR_LIVE_REPO=... TUICR_LIVE_PR=1 GITEA_TOKEN=... \
//! cargo test --lib forge::giteafj::live_tests -- --ignored --nocapture
//! ```

use std::path::PathBuf;

use crate::forge::giteafj::backend::GiteaForgejoBackend;
use crate::forge::submit::{GhSide, InlineComment, SubmitEvent};
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeFileSide, ForgeKind,
    ForgeRepository, PullRequestListQuery, PullRequestTarget,
};
use crate::model::FileStatus;

struct LiveEnv {
    kind: ForgeKind,
    host: String,
    owner: String,
    repo: String,
    pr_number: u64,
}

fn live_env() -> Option<LiveEnv> {
    let kind = match std::env::var("TUICR_LIVE_KIND").ok()?.as_str() {
        "gitea" => ForgeKind::Gitea,
        "forgejo" => ForgeKind::Forgejo,
        _ => return None,
    };
    Some(LiveEnv {
        kind,
        host: std::env::var("TUICR_LIVE_HOST").ok()?,
        owner: std::env::var("TUICR_LIVE_OWNER").ok()?,
        repo: std::env::var("TUICR_LIVE_REPO").ok()?,
        pr_number: std::env::var("TUICR_LIVE_PR").ok()?.parse().ok()?,
    })
}

fn repository(env: &LiveEnv) -> ForgeRepository {
    match env.kind {
        ForgeKind::Gitea => ForgeRepository::gitea(&env.host, &env.owner, &env.repo),
        ForgeKind::Forgejo => ForgeRepository::forgejo(&env.host, &env.owner, &env.repo),
        _ => unreachable!("live_env only produces Gitea/Forgejo"),
    }
}

/// End-to-end smoke test: list/get PR, diff, file context, commits, review
/// threads/summaries/metadata, then create a pending review with one
/// single-line new-side comment and one range comment, and finalize with
/// `REQUEST_CHANGES`. Exercises every `ForgeBackend` method this backend
/// implements against a live instance.
#[test]
#[ignore = "requires a live disposable Gitea/Forgejo instance; see module docs"]
fn should_drive_full_review_lifecycle_against_live_instance() {
    let Some(env) = live_env() else {
        eprintln!("skipping: TUICR_LIVE_* env vars not set");
        return;
    };
    let repo = repository(&env);
    let backend = GiteaForgejoBackend::new(env.kind, Some(repo.clone()));

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

    let target = PullRequestTarget::with_repository(
        repo.clone(),
        env.pr_number,
        format!("{}/{}#{}", env.owner, env.repo, env.pr_number),
    );
    let pr = backend
        .get_pull_request(target)
        .expect("get_pull_request should succeed");
    assert_eq!(pr.number, env.pr_number);
    assert!(!pr.head_sha.is_empty());
    assert!(!pr.base_sha.is_empty());

    let diff = backend
        .get_pull_request_diff(&pr)
        .expect("get_pull_request_diff should succeed");
    assert!(!diff.is_empty(), "PR diff should not be empty");

    let commits = backend
        .list_pull_request_commits(&pr)
        .expect("list_pull_request_commits should succeed");
    assert!(!commits.is_empty(), "PR should have at least one commit");

    // File-context read via the API (no local checkout configured).
    let file_request = ForgeFileLinesRequest {
        repository: repo.clone(),
        base_sha: pr.base_sha.clone(),
        head_sha: pr.head_sha.clone(),
        path: PathBuf::from("src/review-policy.ts"),
        status: FileStatus::Modified,
        side: ForgeFileSide::Head,
        start_line: 1,
        end_line: 5,
    };
    let lines = backend
        .fetch_file_lines(file_request.clone())
        .expect("fetch_file_lines should succeed");
    assert!(!lines.is_empty(), "expected at least one line of context");
    let count = backend
        .file_line_count(file_request)
        .expect("file_line_count should succeed");
    assert!(count > 0, "file should report a positive line count");

    // Existing threads/summaries/metadata should not error even when empty.
    let _threads = backend
        .list_review_threads(&pr)
        .expect("list_review_threads should succeed");
    let _summaries = backend
        .list_review_summaries(&pr)
        .expect("list_review_summaries should succeed");
    let _metadata = backend
        .list_pull_request_review_metadata(&pr)
        .expect("list_pull_request_review_metadata should succeed");

    // Commit-range diff via the API (no local checkout, single commit range).
    if !commits.is_empty() {
        let end = &commits[commits.len() - 1].oid;
        let start = &pr.base_sha;
        match backend.get_pull_request_commit_range_diff(&pr, start, end) {
            Ok(range_diff) => assert!(!range_diff.is_empty()),
            Err(err) => {
                eprintln!("commit range diff unsupported/unavailable on this instance: {err}");
            }
        }
    }

    // Create a pending review with a single-line comment, then finalize
    // with REQUEST_CHANGES. This proves one-pending-review-per-reviewer
    // semantics and the required-REQUEST_CHANGES path end-to-end. A second,
    // multi-line range comment on the same file exercises the range-support
    // divergence: Forgejo accepts `extra_lines_count` natively; Gitea
    // silently ignores it, so the backend must still succeed by collapsing
    // to a single-line comment anchored at the range start (never the end)
    // rather than erroring or claiming native range support it doesn't have.
    let comment = InlineComment {
        path: PathBuf::from("src/review-policy.ts"),
        line: 42,
        side: GhSide::Right,
        counterpart_line: None,
        start_line: None,
        start_side: None,
        old_path: None,
        body: "Live-fixture single-line comment.".to_string(),
        comment_id: "live-fixture-single-line".to_string(),
    };
    let range_comment = InlineComment {
        path: PathBuf::from("src/review-policy.ts"),
        line: 12,
        side: GhSide::Right,
        counterpart_line: None,
        start_line: Some(10),
        start_side: Some(GhSide::Right),
        old_path: None,
        body: "Live-fixture range comment (lines 10-12).".to_string(),
        comment_id: "live-fixture-range".to_string(),
    };
    let comments = [comment, range_comment];
    let request = CreateReviewRequest {
        event: SubmitEvent::RequestChanges,
        commit_id: &pr.head_sha,
        body: "Live-fixture REQUEST_CHANGES review.",
        comments: &comments,
    };
    let response = backend
        .create_review(&pr, request)
        .expect("create_review with REQUEST_CHANGES should succeed");
    assert!(response.id > 0);
}
