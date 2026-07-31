use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::RemoteReviewThread;
use crate::forge::submit::SubmitEvent;
use crate::model::{DiffLine, FileStatus};

/// A forge/host family this tool knows the shape of. Every variant has a
/// working transport (see `crate::forge::registry::create_backend`):
/// `GitHub`/`GitLab` shell out to the existing `gh`/`glab` CLIs;
/// `AzureDevOps` and the `Gitea`/`Forgejo` family use their own HTTP
/// transports (`crate::forge::azure::backend`,
/// `crate::forge::giteafj::backend`). Capability profiles
/// (`crate::forge::capabilities`) and the dry-run publication planner
/// (`crate::forge::dryrun`) describe exactly what each transport actually
/// supports — never a lowest-common-denominator guess. Instantiating a
/// transport for an unregistered kind still always returns a typed error —
/// never a panic, and never a silent fallback to a different kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForgeKind {
    GitHub,
    GitLab,
    /// See `docs/follow-on-map/tickets/12-implement-azure-adapter.md` for
    /// the adapter's implementation/evidence trail.
    AzureDevOps,
    /// Shares route/shape ancestry with `Forgejo`, but ships its own
    /// capability profile: they are one adapter family with divergent
    /// profiles, not one shared profile. See
    /// `docs/follow-on-map/tickets/13-implement-gitea-forgejo-adapter.md`.
    Gitea,
    /// See `Gitea`'s doc comment.
    Forgejo,
}

impl ForgeKind {
    /// Stable lowercase identifier used as both a `provider_mappings` key
    /// (`PersistedThread::upsert_provider_mapping`) and a dry-run
    /// `--provider` CLI value. Kebab-case (`"azure-devops"`), matching the
    /// CLI's own `ForgeKindArg` clap `ValueEnum` rendering — no external
    /// contract requires `snake_case` here, so the CLI's documented form
    /// wins and the dry-run JSON output stays round-trippable with it.
    pub fn provider_key(self) -> &'static str {
        match self {
            ForgeKind::GitHub => "github",
            ForgeKind::GitLab => "gitlab",
            ForgeKind::AzureDevOps => "azure-devops",
            ForgeKind::Gitea => "gitea",
            ForgeKind::Forgejo => "forgejo",
        }
    }

    /// Parse the [`Self::provider_key`] identifier back into a `ForgeKind`.
    /// Accepts the canonical kebab-case form (`"azure-devops"`) and, as a
    /// backward-compatible alias, the snake_case form (`"azure_devops"`)
    /// used by an earlier revision of this API. Returns `None` for anything
    /// else, including this type's own `#[serde(rename_all = "snake_case")]`
    /// derive output (`"git_hub"`, `"git_lab"`) — those two representations
    /// are intentionally distinct and are not interchangeable.
    pub fn from_provider_key(key: &str) -> Option<Self> {
        match key {
            "github" => Some(ForgeKind::GitHub),
            "gitlab" => Some(ForgeKind::GitLab),
            "azure-devops" | "azure_devops" => Some(ForgeKind::AzureDevOps),
            "gitea" => Some(ForgeKind::Gitea),
            "forgejo" => Some(ForgeKind::Forgejo),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeRepository {
    pub kind: ForgeKind,
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl ForgeRepository {
    pub fn github(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: ForgeKind::GitHub,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    pub fn gitlab(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: ForgeKind::GitLab,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// See [`ForgeKind::AzureDevOps`]; `registry::create_backend` builds a
    /// real `AzureDevOpsBackend` transport for a repository of this kind.
    pub fn azure_devops(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: ForgeKind::AzureDevOps,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// See [`ForgeKind::Gitea`]; `registry::create_backend` builds a real
    /// `GiteaForgejoBackend` transport for a repository of this kind.
    pub fn gitea(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: ForgeKind::Gitea,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    /// See [`ForgeKind::Forgejo`]; shares [`Self::gitea`]'s doc comment.
    pub fn forgejo(
        host: impl Into<String>,
        owner: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            kind: ForgeKind::Forgejo,
            host: host.into(),
            owner: owner.into(),
            name: name.into(),
        }
    }

    pub fn slug(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn display_name(&self) -> String {
        if self.host == "github.com" || self.host == "gitlab.com" {
            self.slug()
        } else {
            format!("{}/{}", self.host, self.slug())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestTarget {
    pub repository: Option<ForgeRepository>,
    pub number: u64,
    pub original: String,
}

impl PullRequestTarget {
    pub fn number(number: u64, original: impl Into<String>) -> Self {
        Self {
            repository: None,
            number,
            original: original.into(),
        }
    }

    pub fn with_repository(
        repository: ForgeRepository,
        number: u64,
        original: impl Into<String>,
    ) -> Self {
        Self {
            repository: Some(repository),
            number,
            original: original.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PullRequestListScope {
    #[default]
    Open,
    ReviewRequested,
}

impl PullRequestListScope {
    pub fn toggled(self) -> Self {
        match self {
            Self::Open => Self::ReviewRequested,
            Self::ReviewRequested => Self::Open,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "all",
            Self::ReviewRequested => "requested",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestListQuery {
    pub repository: ForgeRepository,
    pub already_loaded: usize,
    pub page_size: usize,
    pub scope: PullRequestListScope,
}

impl PullRequestListQuery {
    pub fn first_page(repository: ForgeRepository, page_size: usize) -> Self {
        Self::first_page_with_scope(repository, page_size, PullRequestListScope::Open)
    }

    pub fn first_page_with_scope(
        repository: ForgeRepository,
        page_size: usize,
        scope: PullRequestListScope,
    ) -> Self {
        Self {
            repository,
            already_loaded: 0,
            page_size,
            scope,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestSummary {
    pub repository: ForgeRepository,
    pub number: u64,
    pub title: String,
    pub author: Option<String>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedPullRequests {
    pub pull_requests: Vec<PullRequestSummary>,
    pub has_more: bool,
    pub total_loaded: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestDetails {
    pub repository: ForgeRepository,
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub is_draft: bool,
    pub author: Option<String>,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub head_sha: String,
    pub base_sha: String,
    pub body: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub closed: bool,
    pub merged_at: Option<DateTime<Utc>>,
    /// GitLab diff start SHA for inline comment position anchoring.
    /// None for GitHub; populated from `diff_refs.start_sha` for GitLab.
    #[serde(default)]
    pub diff_start_sha: Option<String>,
}

impl PullRequestDetails {
    pub fn is_read_only(&self) -> bool {
        self.closed || self.merged_at.is_some()
    }

    pub fn read_only_reason(&self) -> Option<&'static str> {
        if self.merged_at.is_some() {
            Some("merged")
        } else if self.closed {
            Some("closed")
        } else {
            None
        }
    }
}

/// Stable identity for a PR review session.
///
/// Sessions are keyed by forge kind + host + owner/repo + PR number + head
/// SHA per the spec. Two opens of the same PR at the same head SHA must
/// produce equal keys so persistence reattaches local comments and reviewed
/// markers; a PR that advances to a new head opens a new session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrSessionKey {
    pub repository: ForgeRepository,
    pub number: u64,
    pub head_sha: String,
}

impl PrSessionKey {
    pub fn new(repository: ForgeRepository, number: u64, head_sha: impl Into<String>) -> Self {
        Self {
            repository,
            number,
            head_sha: head_sha.into(),
        }
    }

    pub fn from_details(details: &PullRequestDetails) -> Self {
        Self::new(
            details.repository.clone(),
            details.number,
            details.head_sha.clone(),
        )
    }

    /// Short, human-recognizable head SHA prefix used in filenames and UI.
    pub fn short_head(&self) -> String {
        self.head_sha
            .chars()
            .take(8.min(self.head_sha.len()))
            .collect()
    }

    /// Whether `other` refers to the same PR lineage (forge kind + host +
    /// owner/repo + PR number) as this key, ignoring `head_sha`.
    ///
    /// A PR's head SHA changes every time it gains new commits, but the
    /// underlying review conversation (threads, replies, resolutions) is
    /// still about the same PR. This is an additive helper for
    /// lineage-aware lookups (reusing a review across head advances); it
    /// does not change existing head-sensitive `PartialEq`/`Hash`, which
    /// remain exact-match and continue to drive today's session
    /// open/reattach behavior unchanged.
    pub fn lineage_matches(&self, other: &PrSessionKey) -> bool {
        self.repository == other.repository && self.number == other.number
    }
}

/// Which side of a pull request diff the caller wants to read from.
///
/// Maps to a concrete SHA + path: for added/modified/copied/renamed files
/// the caller wants the head side; for deleted files the base side. Renames
/// pick the old path on the base side and the new path on the head side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForgeFileSide {
    Base,
    Head,
}

/// A single request to read file lines from a forge for context expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgeFileLinesRequest {
    pub repository: ForgeRepository,
    /// Base SHA captured when the PR was opened.
    pub base_sha: String,
    /// Head SHA captured when the PR was opened.
    pub head_sha: String,
    /// File path relative to the repository root.
    pub path: PathBuf,
    /// File status, used to choose the appropriate side without forcing the
    /// caller to also compute it. Renames use `Renamed`; `path` should already
    /// reflect the chosen side.
    pub status: FileStatus,
    /// Which side to read from. The caller is responsible for picking the
    /// right side per the spec mapping rules.
    pub side: ForgeFileSide,
    /// Inclusive 1-based line range. Caller is responsible for clamping.
    pub start_line: u32,
    pub end_line: u32,
}

impl ForgeFileLinesRequest {
    /// Resolve the side and path for a given file based on its status.
    /// Helper for callers that have a `DiffFile` and want to fetch context.
    pub fn side_for_status(status: FileStatus) -> ForgeFileSide {
        match status {
            FileStatus::Deleted => ForgeFileSide::Base,
            FileStatus::Added | FileStatus::Modified | FileStatus::Renamed | FileStatus::Copied => {
                ForgeFileSide::Head
            }
        }
    }

    /// Pick the right path for a forge fetch given old/new paths and the
    /// side. Renamed files use `old_path` on the base side, `new_path` on
    /// the head side.
    pub fn path_for_side(
        side: ForgeFileSide,
        old_path: Option<&PathBuf>,
        new_path: Option<&PathBuf>,
    ) -> Option<PathBuf> {
        match side {
            ForgeFileSide::Base => old_path.or(new_path).cloned(),
            ForgeFileSide::Head => new_path.or(old_path).cloned(),
        }
    }

    /// Return the SHA matching `side`.
    pub fn sha(&self) -> &str {
        match self.side {
            ForgeFileSide::Base => &self.base_sha,
            ForgeFileSide::Head => &self.head_sha,
        }
    }
}

/// Response returned by `ForgeBackend::create_review` after a successful
/// `POST .../pulls/<n>/reviews`. Carries enough state to drive lifecycle
/// writes on the source comments and the success message in the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhCreateReviewResponse {
    /// GitHub's numeric review ID. Stored on each included `Comment` as
    /// `remote_review_id` (stringified).
    pub id: u64,
    /// Web URL of the created review — used in the draft success message so
    /// users can finish the pending review in GitHub.
    pub html_url: String,
    /// Review state as reported by GitHub (`PENDING`, `COMMENTED`,
    /// `APPROVED`, `CHANGES_REQUESTED`). Kept for debugging/logging.
    pub state: String,
}

/// Request to create a review against a PR. The payload is the forge-agnostic
/// shape of the JSON body the backend will POST; downstream the GitHub
/// backend reshapes it via `build_review_payload` and writes it on stdin.
#[derive(Debug, Clone)]
pub struct CreateReviewRequest<'a> {
    pub event: SubmitEvent,
    pub commit_id: &'a str,
    pub body: &'a str,
    pub comments: &'a [crate::forge::submit::InlineComment],
}

/// Request to create a single durable thread — one root comment, posted
/// immediately (not batched into a pending/draft review; see
/// [`ForgeBackend::create_review`] for that path). Mirrors
/// [`crate::model::thread::AnchorTarget`] without depending on the whole
/// [`crate::model::thread::Thread`] type, so backends only need to reason
/// about "what am I anchoring to", not durable-thread bookkeeping.
#[derive(Debug, Clone)]
pub struct NewThreadRequest<'a> {
    /// SHA the comment anchors against. For GitHub this is the PR's current
    /// head commit; for GitLab it feeds the discussion `position`'s
    /// `head_sha` (paired with `base_sha`/`start_sha` already on
    /// [`PullRequestDetails`]).
    pub commit_id: &'a str,
    pub body: &'a str,
    /// `None` for a review-level general comment (no anchor at all).
    pub path: Option<&'a str>,
    /// `None` for a whole-file comment (a path with no line).
    pub line: Option<u32>,
    pub side: Option<crate::model::thread::AnchorSide>,
    /// Present only for a multi-line range anchor; `line`/`side` above then
    /// describe the range's end.
    pub range_start: Option<u32>,
}

/// Response returned after successfully creating a single durable thread.
/// `mapping` is the exact opaque JSON payload the caller should persist via
/// [`crate::model::thread_store::PersistedThread::upsert_provider_mapping`]
/// — every backend is expected to include at least an `"id"` string
/// (matching [`crate::model::thread_store::PersistedThread::has_provider_id`]'s
/// convention) and an `"is_resolved"` boolean (matching
/// [`crate::model::thread_store::thread_from_remote`]'s import shape), so a
/// thread created locally and one imported from the provider always carry
/// the same mapping shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateThreadResponse {
    pub mapping: serde_json::Value,
    /// Provider-native ID of the root comment/note itself, distinct from
    /// the thread/discussion ID in `mapping["id"]` — GitHub's REST review
    /// -comment ID and its owning GraphQL thread ID are different objects;
    /// GitLab's discussion ID and its root note ID are likewise different.
    /// Used as the reply-target for [`ForgeBackend::reply_to_thread`] on
    /// providers (GitHub) whose reply call addresses a comment, not the
    /// thread.
    pub root_comment_id: String,
}

/// Response returned after successfully posting a reply to an existing
/// thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyResponse {
    /// Provider-native ID of the newly created reply comment/note.
    pub comment_id: String,
}

/// A single commit on a pull request, as returned by the forge.
///
/// Fields mirror what the inline commit selector needs to render a row.
/// Backends populate `oid`, `summary`, and `author`; `timestamp` is best-
/// effort (None when the forge does not expose a parseable value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestCommit {
    pub oid: String,
    pub short_oid: String,
    pub summary: String,
    pub author: String,
    pub timestamp: Option<DateTime<Utc>>,
}

/// Minimal review metadata used to infer "commits since my last review".
/// This is separate from displayed review summaries because empty-body
/// approvals still count as reviews for scoping purposes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullRequestReviewMetadata {
    pub viewer_login: Option<String>,
    pub reviews: Vec<PullRequestReviewRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestReviewRecord {
    pub author: Option<String>,
    pub submitted_at: Option<DateTime<Utc>>,
    pub commit_oid: Option<String>,
}

pub trait ForgeBackend {
    fn list_pull_requests(&self, query: PullRequestListQuery) -> Result<PagedPullRequests>;
    fn get_pull_request(&self, target: PullRequestTarget) -> Result<PullRequestDetails>;
    fn get_pull_request_diff(&self, pr: &PullRequestDetails) -> Result<String>;
    /// Fetch the requested file lines from the forge for context expansion.
    /// Implementations may optimize by reading from a local checkout when
    /// available; the trait does not require that path.
    fn fetch_file_lines(&self, request: ForgeFileLinesRequest) -> Result<Vec<DiffLine>>;
    /// Return the total number of lines in a file at the revision described by
    /// `request`. The `start_line` and `end_line` fields of the request are
    /// ignored. Default returns `Ok(0)`; real forge backends override this.
    fn file_line_count(&self, _request: ForgeFileLinesRequest) -> Result<u32> {
        Ok(0)
    }
    /// Fetch existing review discussions for a PR, including their resolved
    /// and outdated state. Implementations should return all threads in
    /// posted order; filtering by visibility happens in the App.
    fn list_review_threads(&self, pr: &PullRequestDetails) -> Result<Vec<RemoteReviewThread>>;
    /// Fetch review-level summary comments — the body text on each
    /// `PullRequestReview`, distinct from line-anchored threads. Default
    /// returns an empty list; only forges with review-summary semantics
    /// (GitHub) need to override.
    fn list_review_summaries(
        &self,
        _pr: &PullRequestDetails,
    ) -> Result<Vec<crate::forge::remote_comments::RemoteReviewSummary>> {
        Ok(Vec::new())
    }
    /// List the commits that make up a pull request, in chronological order
    /// (oldest first; the App reverses to newest-first display order). The
    /// list scopes the inline commit selector so users can narrow a PR's
    /// cumulative diff down to a contiguous subrange.
    fn list_pull_request_commits(&self, pr: &PullRequestDetails) -> Result<Vec<PullRequestCommit>>;
    /// Fetch minimal review metadata for commit-scope inference. Backends
    /// that cannot expose this cheaply can keep the default empty result.
    fn list_pull_request_review_metadata(
        &self,
        _pr: &PullRequestDetails,
    ) -> Result<PullRequestReviewMetadata> {
        Ok(PullRequestReviewMetadata::default())
    }
    /// Fetch the cumulative diff between two commit SHAs that both belong to
    /// `pr`. `start_sha` is the *parent* of the first commit in the
    /// subrange; `end_sha` is the last commit. Implementations may use a
    /// local checkout when both SHAs are present locally, but the source of
    /// truth is the forge.
    fn get_pull_request_commit_range_diff(
        &self,
        pr: &PullRequestDetails,
        start_sha: &str,
        end_sha: &str,
    ) -> Result<String>;
    /// Optional path to a local checkout the backend may consult as an
    /// optimization. The default returns `None`; callers must never treat
    /// this path as the source of truth for PR contents.
    fn local_checkout_path(&self) -> Option<PathBuf> {
        None
    }

    /// Create a review on the PR. The payload-building details (event field
    /// mapping, comment serialization) are the backend's responsibility — the
    /// caller only supplies the high-level inputs. Returns a minimal response
    /// describing the created review (id, html_url, state).
    fn create_review(
        &self,
        pr: &PullRequestDetails,
        request: CreateReviewRequest<'_>,
    ) -> Result<GhCreateReviewResponse>;

    /// Create one durable thread (a single root comment posted immediately,
    /// outside any pending/batch review). This is the write side of
    /// [`Self::list_review_threads`]/durable-thread publication —
    /// `crate::forge::dryrun::plan_publication`'s `CreateThread` operation
    /// executes through this method.
    ///
    /// Default returns [`TuicrError::UnsupportedOperation`]; only backends
    /// with a verified single-comment-thread endpoint (GitHub, GitLab,
    /// Azure DevOps) override it. Gitea/Forgejo remain on this default
    /// until a future ticket adds a verified reply/create-thread route on
    /// that stable pin (see `crate::forge::capabilities::gitea_1_24`'s doc
    /// comment: "no verified reply/resolve route on this stable pin").
    fn create_thread(
        &self,
        _pr: &PullRequestDetails,
        _request: NewThreadRequest<'_>,
    ) -> Result<CreateThreadResponse> {
        Err(TuicrError::UnsupportedOperation(
            "this backend has no create_thread implementation".to_string(),
        ))
    }

    /// Reply to an existing thread. `provider_mapping` is the exact JSON
    /// value previously stored via
    /// `PersistedThread::upsert_provider_mapping` — either from
    /// [`Self::create_thread`]'s `mapping`/`root_comment_id`, or from
    /// importing the thread via [`Self::list_review_threads`]. Backends read
    /// whatever keys they need from it (never body/path heuristics) —
    /// GitHub reads `root_comment_id`; GitLab and Azure DevOps read `id`
    /// (the discussion/thread ID).
    ///
    /// Default returns [`TuicrError::UnsupportedOperation`].
    fn reply_to_thread(
        &self,
        _pr: &PullRequestDetails,
        _provider_mapping: &serde_json::Value,
        _body: &str,
    ) -> Result<ReplyResponse> {
        Err(TuicrError::UnsupportedOperation(
            "this backend has no reply_to_thread implementation".to_string(),
        ))
    }

    /// Set a thread's resolved/unresolved state natively (`resolved: true`
    /// for resolve, `false` for reopen). `provider_mapping` is the same
    /// opaque payload described on [`Self::reply_to_thread`]. Returns the
    /// updated mapping the caller should persist (normally the same value
    /// with `"is_resolved"` flipped) so a subsequent dry-run/execute pass
    /// sees the new state without a separate re-fetch.
    ///
    /// Default returns [`TuicrError::UnsupportedOperation`].
    fn set_thread_resolution(
        &self,
        _pr: &PullRequestDetails,
        _provider_mapping: &serde_json::Value,
        _resolved: bool,
    ) -> Result<serde_json::Value> {
        Err(TuicrError::UnsupportedOperation(
            "this backend has no thread-resolution implementation".to_string(),
        ))
    }

    /// Add one anchored comment to an *already-created* pending (draft)
    /// review, without submitting it — GitHub's incremental pending-review
    /// primitive (`POST
    /// /repos/{owner}/{repo}/pulls/{pull_number}/reviews/{review_id}/comments`),
    /// distinct from [`Self::create_thread`] (a standalone comment posted
    /// immediately, outside any review) and from [`Self::create_review`]
    /// (which creates the pending review itself, optionally with its own
    /// initial batch of comments). `pending_review_id` is the numeric REST
    /// review ID from [`GhCreateReviewResponse::id`] when `create_review`
    /// was called with [`crate::forge::submit::SubmitEvent::Draft`].
    ///
    /// Default returns [`TuicrError::UnsupportedOperation`]; only backends
    /// with a verified add-to-pending-review endpoint (GitHub) override it
    /// — see `capabilities.rs`'s `PendingReviewSupport::incremental()`.
    /// **Not yet wired into [`crate::forge::dryrun`]/[`crate::forge::publish`]'s
    /// per-thread operation graph**: the durable `Thread`/`PersistedThread`
    /// model has no concept yet of "this thread belongs to in-progress
    /// pending review #123" (only [`Self::create_review`]'s legacy batch
    /// path tracks a review ID at all, and only transiently in its own
    /// response). Adding that session-level tracking is a separate,
    /// follow-on concern from this ticket's per-thread reply/resolve
    /// durability; this method exists so the `incremental()` capability
    /// claim is backed by a real, tested transport instead of being
    /// aspirational.
    fn add_comment_to_pending_review(
        &self,
        _pr: &PullRequestDetails,
        _pending_review_id: u64,
        _request: NewThreadRequest<'_>,
    ) -> Result<CreateThreadResponse> {
        Err(TuicrError::UnsupportedOperation(
            "this backend has no add_comment_to_pending_review implementation".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_round_trip_every_provider_key_including_azure_devops_kebab_case() {
        for kind in [
            ForgeKind::GitHub,
            ForgeKind::GitLab,
            ForgeKind::AzureDevOps,
            ForgeKind::Gitea,
            ForgeKind::Forgejo,
        ] {
            let key = kind.provider_key();
            assert_eq!(ForgeKind::from_provider_key(key), Some(kind));
        }
        // The CLI's own clap `ValueEnum` derive renders `AzureDevops` as
        // kebab-case; `provider_key()` must match it exactly, not the
        // `"azure_devops"` snake_case form.
        assert_eq!(ForgeKind::AzureDevOps.provider_key(), "azure-devops");
    }

    #[test]
    fn should_accept_snake_case_azure_devops_as_a_backward_compatible_alias() {
        assert_eq!(
            ForgeKind::from_provider_key("azure_devops"),
            Some(ForgeKind::AzureDevOps)
        );
    }

    #[test]
    fn should_round_trip_pr_session_key_via_serde() {
        // given
        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "agavra", "tuicr"),
            125,
            "abcdef0123456789".to_string(),
        );
        // when
        let serialized = serde_json::to_string(&key).unwrap();
        let restored: PrSessionKey = serde_json::from_str(&serialized).unwrap();
        // then
        assert_eq!(key, restored);
    }

    #[test]
    fn should_truncate_long_head_sha_for_short_head() {
        // given
        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "a", "b"),
            1,
            "1234567890abcdef1234567890abcdef".to_string(),
        );
        // when/then
        assert_eq!(key.short_head(), "12345678");
    }

    #[test]
    fn should_handle_short_head_sha_gracefully() {
        // given
        let key = PrSessionKey::new(
            ForgeRepository::github("github.com", "a", "b"),
            1,
            "abc".to_string(),
        );
        // when/then
        assert_eq!(key.short_head(), "abc");
    }

    #[test]
    fn should_match_lineage_ignoring_head_sha() {
        // given
        let repo = ForgeRepository::github("github.com", "agavra", "tuicr");
        let at_old_head = PrSessionKey::new(repo.clone(), 125, "old-sha".to_string());
        let at_new_head = PrSessionKey::new(repo, 125, "new-sha".to_string());
        // when/then
        assert!(at_old_head.lineage_matches(&at_new_head));
        assert_ne!(at_old_head, at_new_head);
    }

    #[test]
    fn should_not_match_lineage_for_different_pr_number_or_repo() {
        // given
        let repo = ForgeRepository::github("github.com", "agavra", "tuicr");
        let other_repo = ForgeRepository::github("github.com", "agavra", "other");
        let base = PrSessionKey::new(repo.clone(), 125, "sha".to_string());
        let different_number = PrSessionKey::new(repo, 126, "sha".to_string());
        let different_repo = PrSessionKey::new(other_repo, 125, "sha".to_string());
        // when/then
        assert!(!base.lineage_matches(&different_number));
        assert!(!base.lineage_matches(&different_repo));
    }

    #[test]
    fn should_pick_head_side_for_added_modified_renamed_copied() {
        for status in [
            FileStatus::Added,
            FileStatus::Modified,
            FileStatus::Renamed,
            FileStatus::Copied,
        ] {
            assert_eq!(
                ForgeFileLinesRequest::side_for_status(status),
                ForgeFileSide::Head,
                "{status:?} should pick head"
            );
        }
    }

    #[test]
    fn should_pick_base_side_for_deleted_files() {
        assert_eq!(
            ForgeFileLinesRequest::side_for_status(FileStatus::Deleted),
            ForgeFileSide::Base,
        );
    }
}
