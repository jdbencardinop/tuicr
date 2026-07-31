//! Shared Gitea/Forgejo `ForgeBackend` transport.
//!
//! One backend struct parameterized by `ForgeKind::Gitea`/`ForgeKind::Forgejo`
//! rather than two near-duplicate types: every endpoint here is identical
//! wire-for-wire between the two, and the only place their behavior
//! actually diverges (range-comment handling, incremental pending-review
//! comments, request-changes support) is gated through
//! `crate::forge::capabilities::capabilities_for`, which already encodes
//! the evidence-backed difference. See `docs/findings/providers/
//! provider-semantics.md` for the underlying evidence.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{Result, TuicrError};
use crate::forge::capabilities::{ProviderCapabilities, capabilities_for};
use crate::forge::giteafj::auth::{base_url_from_host, resolve_token};
use crate::forge::giteafj::client::{GfHttpClient, require_success};
use crate::forge::giteafj::models::{
    GfCommit, GfCreateReviewComment, GfCreateReviewRequest, GfCreateReviewResponse, GfPullRequest,
    GfReview, GfReviewComment, GfUser,
};
use crate::forge::giteafj::version::{fetch_version, verify_kind_matches};
use crate::forge::remote_comments::{
    RemoteCommentSide, RemoteReviewComment, RemoteReviewSummary, RemoteReviewThread,
};
use crate::forge::submit::{GhSide, InlineComment, SubmitEvent};
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, ForgeFileLinesRequest, ForgeKind, ForgeRepository,
    GhCreateReviewResponse as TraitCreateReviewResponse, PagedPullRequests, PullRequestCommit,
    PullRequestDetails, PullRequestListQuery, PullRequestListScope, PullRequestReviewMetadata,
    PullRequestReviewRecord, PullRequestTarget,
};
use crate::model::DiffLine;
use crate::process::run_command_output;
use crate::vcs::slice_context_lines;

/// Bounded loop guard for every paginated fetch in this module — mirrors
/// the defensive caps already used in `src/forge/github/gh.rs` (e.g. its
/// review-threads GraphQL pagination) so a buggy or malicious server can
/// never hang the caller. 10,000 items at 100/page is far beyond any
/// realistic PR.
const MAX_PAGES: u32 = 100;
const PAGE_LIMIT: u32 = 100;

/// Read a git blob from a local checkout via `git show <sha>:<path>`.
/// Returns `None` (never an error) when the object is missing or the
/// command fails for any reason — callers fall back to the API.
fn read_blob_with_repo(repo_root: &Path, sha: &str, path: &Path) -> Option<String> {
    let spec = format!("{sha}:{}", path.to_string_lossy());
    let exists = run_command_output(
        "git",
        Some(repo_root),
        ["cat-file", "-e", spec.as_str()]
            .iter()
            .map(|s| OsStr::new(*s)),
    );
    if exists.is_err() {
        return None;
    }
    run_command_output(
        "git",
        Some(repo_root),
        ["show", spec.as_str()].iter().map(|s| OsStr::new(*s)),
    )
    .ok()
}

/// `Some(diff)` when both SHAs are present in the local checkout at
/// `repo_root`, via `git diff <start>..<end>`. `None` when either SHA is
/// missing locally or the command fails — callers fall back to the forge's
/// `compare` API.
fn local_range_diff(repo_root: &Path, start_sha: &str, end_sha: &str) -> Option<String> {
    for sha in [start_sha, end_sha] {
        let exists = run_command_output(
            "git",
            Some(repo_root),
            ["cat-file", "-e", sha].iter().map(|s| OsStr::new(*s)),
        );
        if exists.is_err() {
            return None;
        }
    }
    let range = format!("{start_sha}..{end_sha}");
    run_command_output(
        "git",
        Some(repo_root),
        ["diff", range.as_str()].iter().map(|s| OsStr::new(*s)),
    )
    .ok()
}

/// Shared transport for the Gitea/Forgejo family. `kind` fixes which of the
/// two this instance speaks for; every `ForgeRepository` handed to its
/// methods must carry the same `kind` (checked defensively — a mismatch is
/// a caller bug, surfaced as a typed error rather than silently using the
/// wrong capability profile).
#[derive(Debug)]
pub struct GiteaForgejoBackend {
    kind: ForgeKind,
    default_repository: Option<ForgeRepository>,
    /// Optional path to a local checkout. Used only as a read optimization
    /// for file contents and commit-range diffs; the forge remains the
    /// source of truth for PR contents (mirrors `GitHubGhBackend`/
    /// `GitLabGlabBackend`'s documented contract).
    local_checkout: Option<PathBuf>,
    /// Per-host capability cache: a single backend instance may be asked
    /// about more than one repository of the same `kind` across its
    /// lifetime (e.g. `owner/repo#N` targets naming a different repo than
    /// `default_repository`), and each host is version-detected
    /// independently on first use.
    capability_cache: RefCell<HashMap<String, ProviderCapabilities>>,
}

impl GiteaForgejoBackend {
    pub fn new(kind: ForgeKind, default_repository: Option<ForgeRepository>) -> Self {
        Self {
            kind,
            default_repository,
            local_checkout: None,
            capability_cache: RefCell::new(HashMap::new()),
        }
    }

    pub fn with_local_checkout(mut self, checkout: Option<PathBuf>) -> Self {
        self.local_checkout = checkout;
        self
    }

    pub fn set_local_checkout(&mut self, checkout: Option<PathBuf>) {
        self.local_checkout = checkout;
    }

    fn resolve_repository(&self, target: &PullRequestTarget) -> Result<ForgeRepository> {
        let repo = target
            .repository
            .clone()
            .or_else(|| self.default_repository.clone())
            .ok_or_else(|| {
                TuicrError::Forge(format!(
                    "{} pull request target `{}` does not include a repository",
                    self.kind.provider_key(),
                    target.original
                ))
            })?;
        self.check_kind(&repo)?;
        Ok(repo)
    }

    fn check_kind(&self, repo: &ForgeRepository) -> Result<()> {
        if repo.kind != self.kind {
            return Err(TuicrError::Forge(format!(
                "internal error: {} backend was asked to handle a `{}` repository",
                self.kind.provider_key(),
                repo.kind.provider_key()
            )));
        }
        Ok(())
    }

    /// Build a client for `repo` and gate it behind the evidence-backed
    /// version/capability probe (`Self::capabilities_for`) before handing
    /// it back. This runs (and is cached) on the **first** call for any
    /// host, for **every** `ForgeBackend` method — not just `create_review`
    /// — so a version this family has no evidence for (or a
    /// Gitea/Forgejo `kind` mismatch against the live server) is rejected
    /// with a typed error before any read or write operation, per the
    /// "never classify by hostname alone / reject unevidenced versions"
    /// requirement. Remote-URL hostname markers
    /// (`giteafj::detect_self_hosted_kind`) only choose which backend to
    /// *construct*; this probe is the authoritative live check.
    fn client_for(&self, repo: &ForgeRepository) -> Result<GfHttpClient> {
        self.check_kind(repo)?;
        let base_url = base_url_from_host(&repo.host);
        let token = resolve_token(self.kind, &repo.host)?;
        let client = GfHttpClient::new(base_url, token);
        self.capabilities_for(&client, repo)?;
        Ok(client)
    }

    /// Resolve (fetching and caching on first use) the evidence-backed
    /// capability profile for `repo`'s live host/version. Never guesses:
    /// an unrecognized version bucket comes back as a typed
    /// `UnsupportedOperation` from `capabilities_for` itself, and a
    /// Gitea/Forgejo mismatch (wrong `kind` configured for this host) comes
    /// back as one from `verify_kind_matches`.
    fn capabilities_for(
        &self,
        client: &GfHttpClient,
        repo: &ForgeRepository,
    ) -> Result<ProviderCapabilities> {
        if let Some(cached) = self.capability_cache.borrow().get(&repo.host) {
            return Ok(cached.clone());
        }
        let version = fetch_version(client)?;
        verify_kind_matches(self.kind, &version)?;
        let caps = capabilities_for(self.kind, Some(&version))?;
        self.capability_cache
            .borrow_mut()
            .insert(repo.host.clone(), caps.clone());
        Ok(caps)
    }

    fn viewer_login(&self, client: &GfHttpClient) -> Result<String> {
        let user: GfUser = client.get_json("/api/v1/user")?;
        if user.login.is_empty() {
            return Err(TuicrError::Forge(
                "authenticated user response did not include a login".to_string(),
            ));
        }
        Ok(user.login)
    }

    fn base_path(repo: &ForgeRepository) -> String {
        format!("/api/v1/repos/{}/{}", repo.owner, repo.name)
    }

    /// Turn one `limit={page_size}`-bounded page of raw rows into a
    /// `PagedPullRequests`, isolated as a pure function so its `has_more`
    /// math can be unit-tested without a live/mock HTTP round trip.
    /// Gitea/Forgejo's `page`/`limit` pagination has no documented
    /// total-count field or `Link` header this codebase has live evidence
    /// for (see `docs/findings/providers/`), so "received a full page"
    /// (`rows.len() >= page_size`) is the standard, self-correcting
    /// heuristic for "there may be more" — a false positive here only
    /// costs one extra empty-page fetch on the next call, never drops
    /// data. (Comparing a `limit={page_size}`-bounded response's length
    /// against that same bound with `>` — the prior implementation — could
    /// never be true and always reported `has_more: false` past the first
    /// page.)
    fn paginate_open_rows(
        rows: Vec<GfPullRequest>,
        repository: &ForgeRepository,
        page_size: usize,
        already_loaded: usize,
    ) -> PagedPullRequests {
        let has_more = rows.len() >= page_size;
        let pull_requests = rows
            .into_iter()
            .take(page_size)
            .map(|row| row.into_summary(repository))
            .collect::<Vec<_>>();
        let total_loaded = already_loaded + pull_requests.len();
        PagedPullRequests {
            pull_requests,
            has_more,
            total_loaded,
        }
    }

    /// Bounded-pagination fetch of every open PR, used both for the plain
    /// `Open` scope (single page is usually enough, but we still cap
    /// defensively) and as the source list for the client-side
    /// `ReviewRequested` filter (neither Gitea nor Forgejo exposes a
    /// server-side "review requested from me" filter the way GitHub/GitLab
    /// do, so filtering happens here after fetching).
    fn fetch_open_pull_requests(
        &self,
        client: &GfHttpClient,
        repo: &ForgeRepository,
        max_items: usize,
    ) -> Result<Vec<GfPullRequest>> {
        let mut all = Vec::new();
        for page in 1..=MAX_PAGES {
            let path = format!(
                "{}/pulls?state=open&sort=recentupdate&limit={PAGE_LIMIT}&page={page}",
                Self::base_path(repo)
            );
            let rows: Vec<GfPullRequest> = client.get_json(&path)?;
            let received = rows.len();
            all.extend(rows);
            if received < PAGE_LIMIT as usize || all.len() >= max_items {
                break;
            }
        }
        Ok(all)
    }

    fn fetch_reviews(
        &self,
        client: &GfHttpClient,
        repo: &ForgeRepository,
        pr_number: u64,
    ) -> Result<Vec<GfReview>> {
        let mut all = Vec::new();
        for page in 1..=MAX_PAGES {
            let path = format!(
                "{}/pulls/{pr_number}/reviews?limit={PAGE_LIMIT}&page={page}",
                Self::base_path(repo)
            );
            let rows: Vec<GfReview> = client.get_json(&path)?;
            let received = rows.len();
            all.extend(rows);
            if received < PAGE_LIMIT as usize {
                break;
            }
        }
        Ok(all)
    }

    fn fetch_review_comments(
        &self,
        client: &GfHttpClient,
        repo: &ForgeRepository,
        pr_number: u64,
        review_id: u64,
    ) -> Result<Vec<GfReviewComment>> {
        let path = format!(
            "{}/pulls/{pr_number}/reviews/{review_id}/comments",
            Self::base_path(repo)
        );
        client.get_json(&path)
    }

    fn fetch_file_via_api(&self, request: &ForgeFileLinesRequest) -> Result<String> {
        let client = self.client_for(&request.repository)?;
        let path_str = request.path.to_string_lossy().replace('\\', "/");
        let endpoint = format!(
            "{}/raw/{}?ref={}",
            Self::base_path(&request.repository),
            path_str,
            request.sha(),
        );
        let response = client.get(&endpoint)?;
        require_success(&response, &endpoint)?;
        Ok(response.body)
    }

    /// Find the current viewer's own `PENDING` review on `pr`, if any —
    /// the "one pending review per reviewer/PR" invariant this family
    /// (unlike GitHub) requires the client to enforce itself, since a
    /// second `POST .../reviews` call does not implicitly reuse or merge
    /// with an already-pending one.
    fn find_own_pending_review(
        &self,
        client: &GfHttpClient,
        repo: &ForgeRepository,
        pr_number: u64,
        viewer_login: &str,
    ) -> Result<Option<GfReview>> {
        let reviews = self.fetch_reviews(client, repo, pr_number)?;
        Ok(reviews.into_iter().find(|r| {
            r.state == "PENDING" && r.user.as_ref().is_some_and(|u| u.login == viewer_login)
        }))
    }

    /// Best-effort duplicate guard for the incremental-add-comment retry
    /// path: fetch the pending review's current comments and compare by
    /// `(path, position/original_position, body)` so a retried
    /// `create_review` call after a partial failure does not re-post
    /// comments the previous attempt already got through. Neither Gitea
    /// nor Forgejo documents a server-side idempotency key for comment
    /// creation (`capabilities.create_idempotency` is `false` for every
    /// profile in this codebase), so this content match is the practical
    /// "where possible" mitigation rather than a guaranteed one.
    fn existing_comment_signatures(
        &self,
        client: &GfHttpClient,
        repo: &ForgeRepository,
        pr_number: u64,
        review_id: u64,
    ) -> Result<Vec<(String, u64, u64, String)>> {
        let comments = self.fetch_review_comments(client, repo, pr_number, review_id)?;
        Ok(comments
            .into_iter()
            .map(|c| (c.path, c.position, c.original_position, c.body))
            .collect())
    }

    fn build_create_review_comment(
        comment: &InlineComment,
        range: crate::forge::capabilities::RangeSupport,
    ) -> GfCreateReviewComment {
        use crate::forge::capabilities::RangeSupport;

        // Gitea/Forgejo have no separate start/end fields: a single
        // `position` (this side's anchor line) plus, on providers that
        // accept it, `extra_lines_count` (how many further lines the range
        // extends). Per the dry-run emulation policy for
        // `RangeSupport::None` (`crate::forge::dryrun::anchor_outcome`),
        // the anchor is always the range's *start* line, never its end —
        // this keeps write-path behavior identical to what Gitea itself
        // does when it silently drops `extra_lines_count`
        // (`extra_lines_count_behavior: "ignored"` in
        // `fixtures/providers/results-examples/gitea-1.24.7.example.json`):
        // a single-line comment anchored at the range start.
        let anchor_line = comment.start_line.unwrap_or(comment.line);
        let (old_position, new_position) = match comment.side {
            GhSide::Left => (anchor_line as i64, 0),
            GhSide::Right => (0, anchor_line as i64),
        };
        let extra_lines_count = match range {
            RangeSupport::SameSide => comment
                .start_line
                .map(|start| (comment.line.saturating_sub(start)) as i64),
            _ => None,
        };
        GfCreateReviewComment {
            path: comment.path.to_string_lossy().to_string(),
            body: comment.body.clone(),
            old_position,
            new_position,
            extra_lines_count,
        }
    }

    fn event_literal(event: SubmitEvent) -> &'static str {
        match event {
            SubmitEvent::Comment => "COMMENT",
            SubmitEvent::Approve => "APPROVED",
            SubmitEvent::RequestChanges => "REQUEST_CHANGES",
            SubmitEvent::Draft => "PENDING",
        }
    }
}

impl ForgeBackend for GiteaForgejoBackend {
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests> {
        self.check_kind(&query.repository)?;
        let client = self.client_for(&query.repository)?;
        let page_size = query.page_size.max(1);

        match query.scope {
            PullRequestListScope::Open => {
                let page = (query.already_loaded / page_size) as u32 + 1;
                let path = format!(
                    "{}/pulls?state=open&sort=recentupdate&limit={page_size}&page={page}",
                    Self::base_path(&query.repository)
                );
                let rows: Vec<GfPullRequest> = client.get_json(&path)?;
                Ok(Self::paginate_open_rows(
                    rows,
                    &query.repository,
                    page_size,
                    query.already_loaded,
                ))
            }
            PullRequestListScope::ReviewRequested => {
                let viewer = self.viewer_login(&client)?;
                let all = self.fetch_open_pull_requests(
                    &client,
                    &query.repository,
                    (MAX_PAGES as usize) * (PAGE_LIMIT as usize),
                )?;
                let filtered: Vec<GfPullRequest> = all
                    .into_iter()
                    .filter(|pr| pr.requests_review_from(&viewer))
                    .collect();
                let has_more = filtered.len() > query.already_loaded + page_size;
                let pull_requests = filtered
                    .into_iter()
                    .skip(query.already_loaded)
                    .take(page_size)
                    .map(|row| row.into_summary(&query.repository))
                    .collect::<Vec<_>>();
                let total_loaded = query.already_loaded + pull_requests.len();
                Ok(PagedPullRequests {
                    pull_requests,
                    has_more,
                    total_loaded,
                })
            }
        }
    }

    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails> {
        let repository = self.resolve_repository(&target)?;
        let client = self.client_for(&repository)?;
        let path = format!("{}/pulls/{}", Self::base_path(&repository), target.number);
        let pr: GfPullRequest = client.get_json(&path)?;
        pr.into_details(&repository)
    }

    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<String> {
        let client = self.client_for(&pr.repository)?;
        let path = format!(
            "{}/pulls/{}.diff",
            Self::base_path(&pr.repository),
            pr.number
        );
        let response = client.get(&path)?;
        require_success(&response, &path)?;
        Ok(response.body)
    }

    fn local_checkout_path(&self) -> Option<PathBuf> {
        self.local_checkout.clone()
    }

    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>> {
        if request.start_line == 0 || request.start_line > request.end_line {
            return Ok(Vec::new());
        }
        let local_content = self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()));
        let content = match local_content {
            Some(content) => content,
            None => self.fetch_file_via_api(&request)?,
        };
        Ok(slice_context_lines(
            &content,
            request.start_line,
            request.end_line,
        ))
    }

    fn file_line_count(&self, request: ForgeFileLinesRequest) -> Result<u32> {
        let local_content = self
            .local_checkout
            .as_deref()
            .and_then(|root| read_blob_with_repo(root, request.sha(), request.path.as_path()));
        let content = match local_content {
            Some(content) => content,
            None => self.fetch_file_via_api(&request)?,
        };
        Ok(content.lines().count() as u32)
    }

    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>> {
        let client = self.client_for(&pr.repository)?;
        let reviews = self.fetch_reviews(&client, &pr.repository, pr.number)?;
        let mut threads = Vec::new();
        for review in &reviews {
            let comments =
                self.fetch_review_comments(&client, &pr.repository, pr.number, review.id)?;
            for comment in comments {
                let (line, side) = if comment.position > 0 {
                    (Some(comment.position as u32), RemoteCommentSide::Right)
                } else if comment.original_position > 0 {
                    (
                        Some(comment.original_position as u32),
                        RemoteCommentSide::Left,
                    )
                } else {
                    (None, RemoteCommentSide::Right)
                };
                threads.push(RemoteReviewThread {
                    id: comment.id.to_string(),
                    path: comment.path.clone(),
                    line,
                    side,
                    is_resolved: comment.resolver.is_some(),
                    // Requirement: a review's explicit `stale` boolean
                    // covers all of its comments (see
                    // `StaleAnchorSignal::ExplicitReviewFlag` in
                    // `capabilities.rs`) — Gitea/Forgejo do not track
                    // staleness per comment.
                    is_outdated: review.stale,
                    comments: vec![RemoteReviewComment {
                        id: comment.id.to_string(),
                        author: comment.user.map(|u| u.login),
                        body: comment.body,
                        created_at: comment.created_at,
                        // Neither stable pin exposes a working reply
                        // mechanism (`ReplySupport::Unsupported`), so every
                        // comment here is necessarily a thread root, never
                        // a reply.
                        in_reply_to: None,
                        url: comment.html_url,
                    }],
                });
            }
        }
        Ok(threads)
    }

    fn list_review_summaries(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewSummary>> {
        let client = self.client_for(&pr.repository)?;
        let reviews = self.fetch_reviews(&client, &pr.repository, pr.number)?;
        Ok(reviews
            .into_iter()
            .filter_map(GfReview::into_summary)
            .collect())
    }

    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>> {
        let client = self.client_for(&pr.repository)?;
        let mut commits = Vec::new();
        for page in 1..=MAX_PAGES {
            let path = format!(
                "{}/pulls/{}/commits?limit={PAGE_LIMIT}&page={page}",
                Self::base_path(&pr.repository),
                pr.number
            );
            let rows: Vec<GfCommit> = client.get_json(&path)?;
            let received = rows.len();
            commits.extend(rows.into_iter().map(GfCommit::into_pull_request_commit));
            if received < PAGE_LIMIT as usize {
                break;
            }
        }
        Ok(commits)
    }

    fn list_pull_request_review_metadata(
        &self,
        pr: &PullRequestDetails,
    ) -> Result<PullRequestReviewMetadata> {
        let client = self.client_for(&pr.repository)?;
        let viewer_login = self.viewer_login(&client).ok();
        let reviews = self.fetch_reviews(&client, &pr.repository, pr.number)?;
        let records = reviews
            .into_iter()
            .map(|r| PullRequestReviewRecord {
                author: r.user.map(|u| u.login),
                submitted_at: r.submitted_at,
                commit_oid: if r.commit_id.is_empty() {
                    None
                } else {
                    Some(r.commit_id)
                },
            })
            .collect();
        Ok(PullRequestReviewMetadata {
            viewer_login,
            reviews: records,
        })
    }

    fn get_pull_request_commit_range_diff(
        &self,
        pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<String> {
        if let Some(root) = self.local_checkout.as_deref()
            && let Some(diff) = local_range_diff(root, start_sha, end_sha)
        {
            return Ok(diff);
        }

        let client = self.client_for(&pr.repository)?;
        let path = format!(
            "{}/compare/{start_sha}...{end_sha}?output=diff",
            Self::base_path(&pr.repository)
        );
        let response = client.get(&path)?;
        if response.is_success() {
            return Ok(response.body);
        }
        // Not evidence-backed as present on the exact stable pins this
        // codebase targets (the upstream `output=diff` query parameter is
        // a relatively recent addition) — treat any failure as a typed
        // unsupported-operation rather than guessing at another endpoint
        // shape.
        Err(TuicrError::UnsupportedOperation(format!(
            "{} does not support commit-range diffing outside a local checkout on this host \
             (compare endpoint returned HTTP {})",
            self.kind.provider_key(),
            response.status
        )))
    }

    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<TraitCreateReviewResponse> {
        use crate::forge::capabilities::RequestChangesSupport;

        let client = self.client_for(&pr.repository)?;
        let caps = self.capabilities_for(&client, &pr.repository)?;

        // Defensive guard: every current profile for this family is
        // `RequestChangesSupport::Native`, but never silently downgrade a
        // required REQUEST_CHANGES to a plain comment if that ever stops
        // being true — fail explicitly per the "Required REQUEST_CHANGES
        // failures must fail explicitly" requirement.
        if matches!(request.event, SubmitEvent::RequestChanges)
            && !matches!(caps.request_changes, RequestChangesSupport::Native)
        {
            return Err(TuicrError::UnsupportedOperation(format!(
                "{} does not support REQUEST_CHANGES as a native review event on this host",
                self.kind.provider_key()
            )));
        }

        let viewer_login = self.viewer_login(&client)?;
        let existing_pending =
            self.find_own_pending_review(&client, &pr.repository, pr.number, &viewer_login)?;

        let new_comments: Vec<GfCreateReviewComment> = request
            .comments
            .iter()
            .map(|c| Self::build_create_review_comment(c, caps.range))
            .collect();

        let Some(existing) = existing_pending else {
            // No pre-existing pending review by this viewer: create fresh,
            // in one shot, with every requested comment attached.
            let body = GfCreateReviewRequest {
                event: Self::event_literal(request.event),
                body: request.body.to_string(),
                commit_id: request.commit_id.to_string(),
                comments: new_comments,
            };
            let path = format!(
                "{}/pulls/{}/reviews",
                Self::base_path(&pr.repository),
                pr.number
            );
            let created: GfCreateReviewResponse = client.post_json(&path, &body)?;
            return Ok(TraitCreateReviewResponse {
                id: created.id,
                html_url: created.html_url,
                state: created.state,
            });
        };

        // A pending review by this viewer already exists on the server —
        // enforce "one pending review per reviewer/PR" by extending it
        // instead of creating a second one.
        if !new_comments.is_empty() {
            if !caps.pending_review.incremental_comments {
                return Err(TuicrError::Forge(format!(
                    "{} already has a pending review for this PR and cannot add more comments \
                     to it (no incremental add-comment route on this host); submit or discard \
                     the existing pending review first",
                    self.kind.provider_key()
                )));
            }

            let already_present =
                self.existing_comment_signatures(&client, &pr.repository, pr.number, existing.id)?;
            let add_comment_path = format!(
                "{}/pulls/{}/reviews/{}/comments",
                Self::base_path(&pr.repository),
                pr.number,
                existing.id
            );
            for comment in &new_comments {
                let signature = (
                    comment.path.clone(),
                    comment.new_position as u64,
                    comment.old_position as u64,
                    comment.body.clone(),
                );
                if already_present.contains(&signature) {
                    // Already posted on a previous (partially-failed)
                    // attempt — skip to avoid a duplicate.
                    continue;
                }
                let _: serde_json::Value = client.post_json(&add_comment_path, comment)?;
            }
        }

        if matches!(request.event, SubmitEvent::Draft) {
            // Draft just means "keep it pending" — comments (if any) are
            // already attached above; nothing further to submit.
            return Ok(TraitCreateReviewResponse {
                id: existing.id,
                html_url: existing.html_url,
                state: existing.state,
            });
        }

        // Finalize the existing pending review.
        let submit_path = format!(
            "{}/pulls/{}/reviews/{}",
            Self::base_path(&pr.repository),
            pr.number,
            existing.id
        );
        let submit_body = serde_json::json!({
            "event": Self::event_literal(request.event),
            "body": request.body,
        });
        let submitted: GfCreateReviewResponse = client.post_json(&submit_path, &submit_body)?;
        Ok(TraitCreateReviewResponse {
            id: submitted.id,
            html_url: submitted.html_url,
            state: submitted.state,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_reject_repository_with_mismatched_kind() {
        let backend = GiteaForgejoBackend::new(ForgeKind::Gitea, None);
        let repo = ForgeRepository::forgejo("http://example.com", "o", "r");
        let err = backend.check_kind(&repo).unwrap_err();
        assert!(matches!(err, TuicrError::Forge(_)));
    }

    #[test]
    fn should_accept_repository_with_matching_kind() {
        let backend = GiteaForgejoBackend::new(ForgeKind::Forgejo, None);
        let repo = ForgeRepository::forgejo("http://example.com", "o", "r");
        assert!(backend.check_kind(&repo).is_ok());
    }

    #[test]
    fn should_fail_resolve_repository_without_target_or_default() {
        let backend = GiteaForgejoBackend::new(ForgeKind::Gitea, None);
        let target = PullRequestTarget::number(5, "5");
        assert!(backend.resolve_repository(&target).is_err());
    }

    #[test]
    fn should_resolve_repository_from_default_when_target_has_none() {
        let repo = ForgeRepository::gitea("http://example.com", "o", "r");
        let backend = GiteaForgejoBackend::new(ForgeKind::Gitea, Some(repo.clone()));
        let target = PullRequestTarget::number(5, "5");
        assert_eq!(backend.resolve_repository(&target).unwrap(), repo);
    }

    #[test]
    fn should_prefer_target_repository_over_default() {
        let default_repo = ForgeRepository::gitea("http://example.com", "default-owner", "r");
        let target_repo = ForgeRepository::gitea("http://example.com", "target-owner", "r");
        let backend = GiteaForgejoBackend::new(ForgeKind::Gitea, Some(default_repo));
        let target = PullRequestTarget::with_repository(target_repo.clone(), 5, "target-owner/r#5");
        assert_eq!(backend.resolve_repository(&target).unwrap(), target_repo);
    }

    #[test]
    fn should_build_single_line_comment_with_no_extra_lines_count() {
        let comment = InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line: 10,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: None,
            start_side: None,
            old_path: None,
            body: "nit".to_string(),
            comment_id: "c1".to_string(),
        };
        let built = GiteaForgejoBackend::build_create_review_comment(
            &comment,
            crate::forge::capabilities::RangeSupport::SameSide,
        );
        assert_eq!(built.new_position, 10);
        assert_eq!(built.old_position, 0);
        assert_eq!(built.extra_lines_count, None);
    }

    #[test]
    fn should_collapse_range_comment_to_start_line_when_range_unsupported() {
        let comment = InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line: 20,
            side: GhSide::Right,
            counterpart_line: None,
            start_line: Some(15),
            start_side: Some(GhSide::Right),
            old_path: None,
            body: "range nit".to_string(),
            comment_id: "c2".to_string(),
        };
        let built = GiteaForgejoBackend::build_create_review_comment(
            &comment,
            crate::forge::capabilities::RangeSupport::None,
        );
        assert_eq!(
            built.new_position, 15,
            "must anchor at range start, not end"
        );
        assert_eq!(
            built.extra_lines_count, None,
            "must never claim native range"
        );
    }

    #[test]
    fn should_send_extra_lines_count_anchored_at_start_when_range_supported() {
        let comment = InlineComment {
            path: PathBuf::from("src/lib.rs"),
            line: 20,
            side: GhSide::Left,
            counterpart_line: None,
            start_line: Some(15),
            start_side: Some(GhSide::Left),
            old_path: None,
            body: "range nit".to_string(),
            comment_id: "c3".to_string(),
        };
        let built = GiteaForgejoBackend::build_create_review_comment(
            &comment,
            crate::forge::capabilities::RangeSupport::SameSide,
        );
        assert_eq!(built.old_position, 15);
        assert_eq!(built.new_position, 0);
        assert_eq!(built.extra_lines_count, Some(5));
    }

    #[test]
    fn should_map_submit_events_to_provider_state_literals() {
        assert_eq!(
            GiteaForgejoBackend::event_literal(SubmitEvent::Comment),
            "COMMENT"
        );
        assert_eq!(
            GiteaForgejoBackend::event_literal(SubmitEvent::Approve),
            "APPROVED"
        );
        assert_eq!(
            GiteaForgejoBackend::event_literal(SubmitEvent::RequestChanges),
            "REQUEST_CHANGES"
        );
        assert_eq!(
            GiteaForgejoBackend::event_literal(SubmitEvent::Draft),
            "PENDING"
        );
    }

    #[test]
    fn should_build_base_path_from_owner_and_name() {
        let repo = ForgeRepository::gitea("http://example.com", "owner", "repo");
        assert_eq!(
            GiteaForgejoBackend::base_path(&repo),
            "/api/v1/repos/owner/repo"
        );
    }

    #[test]
    fn should_report_has_more_when_a_full_page_is_received() {
        let repo = ForgeRepository::gitea("http://example.com", "owner", "repo");
        let row_json =
            r#"{"number":1,"base":{"ref":"main","sha":"a"},"head":{"ref":"feat","sha":"b"}}"#;
        let rows: Vec<GfPullRequest> = vec![
            serde_json::from_str(row_json).unwrap(),
            serde_json::from_str(row_json).unwrap(),
        ];
        let page = GiteaForgejoBackend::paginate_open_rows(rows, &repo, 2, 0);
        assert_eq!(page.pull_requests.len(), 2);
        assert_eq!(page.total_loaded, 2);
        assert!(
            page.has_more,
            "a full page (2 rows for page_size 2) must signal has_more, \
             even though the server was asked for exactly limit=page_size"
        );
    }

    #[test]
    fn should_report_no_more_when_a_partial_page_is_received() {
        let repo = ForgeRepository::gitea("http://example.com", "owner", "repo");
        let row: GfPullRequest = serde_json::from_str(
            r#"{"number":1,"base":{"ref":"main","sha":"a"},"head":{"ref":"feat","sha":"b"}}"#,
        )
        .unwrap();
        let page = GiteaForgejoBackend::paginate_open_rows(vec![row], &repo, 5, 10);
        assert_eq!(page.pull_requests.len(), 1);
        assert_eq!(page.total_loaded, 11);
        assert!(
            !page.has_more,
            "a short page (1 row for page_size 5) is the last page"
        );
    }

    #[test]
    fn should_reject_unevidenced_version_before_any_read_or_write_operation() {
        use crate::forge::giteafj::client::GfHttpClient;
        use crate::forge::giteafj::test_support::start_mock_server;
        use std::collections::HashMap;

        let mut responses = HashMap::new();
        responses.insert(
            "/api/v1/version".to_string(),
            (200u16, r#"{"version":"1.20.0"}"#.to_string()),
        );
        let base_url = start_mock_server(responses);

        let backend = GiteaForgejoBackend::new(ForgeKind::Gitea, None);
        let repo = ForgeRepository::gitea(&base_url, "owner", "repo");
        let client = GfHttpClient::new(base_url, "mock-token".to_string());

        // This is the same gate `client_for` now runs before every
        // `ForgeBackend` method (list/get/diff/commits/threads/create_review
        // all call `client_for` first) — proving it here proves every one
        // of them refuses an unevidenced version rather than guessing.
        let err = backend.capabilities_for(&client, &repo).unwrap_err();
        assert!(
            matches!(err, TuicrError::UnsupportedOperation(_)),
            "unevidenced version 1.20.0 must be a typed UnsupportedOperation, not silently \
             accepted: {err:?}"
        );
    }

    #[test]
    fn should_reject_kind_mismatch_against_live_forgejo_fingerprint() {
        use crate::forge::giteafj::client::GfHttpClient;
        use crate::forge::giteafj::test_support::start_mock_server;
        use std::collections::HashMap;

        let mut responses = HashMap::new();
        responses.insert(
            "/api/v1/version".to_string(),
            (200u16, r#"{"version":"16.0.1+gitea-1.22.0"}"#.to_string()),
        );
        let base_url = start_mock_server(responses);

        // Configured as Gitea, but the live server is Forgejo-fingerprinted.
        let backend = GiteaForgejoBackend::new(ForgeKind::Gitea, None);
        let repo = ForgeRepository::gitea(&base_url, "owner", "repo");
        let client = GfHttpClient::new(base_url, "mock-token".to_string());

        let err = backend.capabilities_for(&client, &repo).unwrap_err();
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_accept_and_cache_evidenced_version_across_repeated_calls() {
        use crate::forge::giteafj::client::GfHttpClient;
        use crate::forge::giteafj::test_support::start_mock_server;
        use std::collections::HashMap;

        // Exactly one `/api/v1/version` response is registered; the mock
        // server thread exits after serving it once. If `capabilities_for`
        // re-fetched on the second call instead of using
        // `capability_cache`, the second call would fail to connect
        // (listener already dropped) instead of returning `Ok` again.
        let mut responses = HashMap::new();
        responses.insert(
            "/api/v1/version".to_string(),
            (200u16, r#"{"version":"1.24.7"}"#.to_string()),
        );
        let base_url = start_mock_server(responses);

        let backend = GiteaForgejoBackend::new(ForgeKind::Gitea, None);
        let repo = ForgeRepository::gitea(&base_url, "owner", "repo");
        let client = GfHttpClient::new(base_url, "mock-token".to_string());

        let first = backend
            .capabilities_for(&client, &repo)
            .expect("first call");
        let second = backend
            .capabilities_for(&client, &repo)
            .expect("second call must hit the cache, not the network");
        assert_eq!(first.range, second.range);
    }
}
