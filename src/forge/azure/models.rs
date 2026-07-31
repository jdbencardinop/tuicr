//! Wire models for the Azure DevOps Git REST API surface (`api-version=7.1`).
//!
//! Field names and JSON shapes are taken from official Microsoft Learn REST
//! API reference pages (primary evidence) plus, for a handful of nested
//! fields Microsoft Learn's rendered property tables truncate,
//! `docs.rs/azure_devops_rust_api` — a crate generated directly from
//! Microsoft's own published OpenAPI/Swagger specification (secondary,
//! OpenAPI-spec-derived evidence, not itself an official Microsoft
//! document). See `src/forge/azure/fixtures/README.md` for the exact page
//! citations per endpoint/type. Every field here is either directly
//! evidenced or a documented optional field defaulted via `#[serde(default)]`
//! — nothing is invented.
//!
//! Primary references:
//! - Pull Requests - Get:
//!   <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get>
//! - Pull Requests - Get Pull Requests (list):
//!   <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get-pull-requests>
//! - Pull Request Iterations - List / Get:
//!   <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-iterations/list>
//! - Pull Request Iterations - Get Iteration Changes:
//!   <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-iterations/get-iteration-changes>
//! - Pull Request Threads - List / Create / Update:
//!   <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-threads>
//! - Pull Request Thread Comments - Create:
//!   <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-thread-comments/create>
//! - Pull Request Reviewers - Create (vote):
//!   <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-reviewers/create-pull-request-reviewer>
//! - Pull Request Commits (continuation-token pagination):
//!   <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-commits/get-pull-request-commits>

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TuicrError};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestSummary,
};

/// `GitRepository.project` — only the fields this adapter uses.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoTeamProjectReference {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// `GitRepository` (nested under `GitPullRequest.repository`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoGitRepository {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub project: Option<AdoTeamProjectReference>,
}

/// `IdentityRef` — the common identity shape reused across `createdBy`,
/// comment `author`, and (via [`AdoIdentityRefWithVote`]) `reviewers`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoIdentityRef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub unique_name: String,
    #[serde(default)]
    pub url: String,
}

/// `IdentityRefWithVote` — `GitPullRequest.reviewers[]` entries. Vote value
/// semantics (evidenced): `10` = approved, `5` = approved with suggestions,
/// `0` = no vote, `-5` = waiting for author, `-10` = rejected. See
/// <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-requests/get#identityrefwithvote>.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoIdentityRefWithVote {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub unique_name: String,
    #[serde(default)]
    pub vote: i32,
    #[serde(default)]
    pub is_required: bool,
}

/// A single reviewer vote value, per [`AdoIdentityRefWithVote::vote`]'s
/// documented semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdoVote {
    Approved,
    ApprovedWithSuggestions,
    NoVote,
    WaitingForAuthor,
    Rejected,
}

impl AdoVote {
    /// The literal integer Azure DevOps expects in a reviewer PUT/POST
    /// body's `vote` field.
    pub fn wire_value(self) -> i32 {
        match self {
            AdoVote::Approved => 10,
            AdoVote::ApprovedWithSuggestions => 5,
            AdoVote::NoVote => 0,
            AdoVote::WaitingForAuthor => -5,
            AdoVote::Rejected => -10,
        }
    }

    pub fn from_wire_value(value: i32) -> Option<Self> {
        match value {
            10 => Some(AdoVote::Approved),
            5 => Some(AdoVote::ApprovedWithSuggestions),
            0 => Some(AdoVote::NoVote),
            -5 => Some(AdoVote::WaitingForAuthor),
            -10 => Some(AdoVote::Rejected),
            _ => None,
        }
    }
}

/// `GitPullRequest.status`/`mergeStatus` (`PullRequestStatus`/
/// `PullRequestAsyncStatus`) render as lowercase strings on the wire
/// (`"active"`, `"abandoned"`, `"completed"`, `"notSet"`, `"conflicts"`,
/// ...); this adapter only distinguishes what it needs (open vs. not) and
/// keeps the raw string for display, so no enum is modeled here — see
/// `AdoPullRequest::into_details`.

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoPullRequest {
    pub pull_request_id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub is_draft: bool,
    #[serde(default)]
    pub created_by: Option<AdoIdentityRef>,
    #[serde(default)]
    pub creation_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub source_ref_name: String,
    #[serde(default)]
    pub target_ref_name: String,
    #[serde(default)]
    pub merge_status: String,
    #[serde(default)]
    pub last_merge_source_commit: Option<AdoCommitRefShort>,
    #[serde(default)]
    pub last_merge_target_commit: Option<AdoCommitRefShort>,
    #[serde(default)]
    pub repository: Option<AdoGitRepository>,
    #[serde(default)]
    pub reviewers: Vec<AdoIdentityRefWithVote>,
    #[serde(default)]
    pub url: String,
}

/// `GitCommitRef` as embedded in `lastMergeSourceCommit`/
/// `lastMergeTargetCommit` — only `commitId` is documented as always
/// present in that embedded position.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCommitRefShort {
    #[serde(default)]
    pub commit_id: String,
}

fn require_field(value: &str, name: &str) -> Result<()> {
    if value.is_empty() {
        return Err(TuicrError::Forge(format!(
            "Azure DevOps pull request response is missing required field `{name}`"
        )));
    }
    Ok(())
}

/// Strip the `refs/heads/` prefix Azure DevOps always includes in
/// `sourceRefName`/`targetRefName`, matching how GitHub/GitLab's
/// equivalents already surface a bare branch name in `PullRequestDetails`.
fn strip_ref_prefix(git_ref: &str) -> String {
    git_ref
        .strip_prefix("refs/heads/")
        .unwrap_or(git_ref)
        .to_string()
}

impl AdoPullRequest {
    pub fn into_summary(self, repository: &ForgeRepository) -> PullRequestSummary {
        PullRequestSummary {
            repository: repository.clone(),
            number: self.pull_request_id,
            title: self.title,
            author: self.created_by.map(|u| u.display_name),
            head_ref_name: strip_ref_prefix(&self.source_ref_name),
            base_ref_name: strip_ref_prefix(&self.target_ref_name),
            updated_at: self.creation_date,
            url: self.url,
            state: self.status.clone(),
            is_draft: self.is_draft,
        }
    }

    pub fn into_details(self, repository: &ForgeRepository) -> Result<PullRequestDetails> {
        let head_sha = self
            .last_merge_source_commit
            .as_ref()
            .map(|c| c.commit_id.clone())
            .unwrap_or_default();
        let base_sha = self
            .last_merge_target_commit
            .as_ref()
            .map(|c| c.commit_id.clone())
            .unwrap_or_default();
        require_field(&head_sha, "lastMergeSourceCommit.commitId")?;
        require_field(&base_sha, "lastMergeTargetCommit.commitId")?;
        let closed = self.status != "active";
        let merged_at = if self.status == "completed" {
            self.closed_date
        } else {
            None
        };
        Ok(PullRequestDetails {
            repository: repository.clone(),
            number: self.pull_request_id,
            title: self.title,
            url: self.url,
            state: self.status,
            is_draft: self.is_draft,
            author: self.created_by.map(|u| u.display_name),
            head_ref_name: strip_ref_prefix(&self.source_ref_name),
            base_ref_name: strip_ref_prefix(&self.target_ref_name),
            head_sha,
            base_sha,
            body: self.description,
            updated_at: self.creation_date,
            closed,
            merged_at,
            // Azure DevOps has no `diff_refs`-style separate "start" SHA —
            // its diff model is iteration-based (`AdoIteration`), not a
            // GitLab-style version triple. `None` here is correct, not a
            // gap: `diff_start_sha` is a GitLab-only concept per its own
            // doc comment in `crate::forge::traits::PullRequestDetails`.
            diff_start_sha: None,
        })
    }
}

/// `GitPullRequestIteration` — one revision of the PR's diff.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoIteration {
    pub id: u32,
    #[serde(default)]
    pub source_ref_commit: Option<AdoCommitRefShort>,
    #[serde(default)]
    pub common_ref_commit: Option<AdoCommitRefShort>,
    #[serde(default)]
    pub created_date: Option<DateTime<Utc>>,
}

/// `GitPullRequestIterationChanges` — one page of file-level changes for a
/// given iteration (or iteration-pair). Paginated via `$top`/`$skip` query
/// parameters (distinct from Commits' continuation-token mechanism); see
/// module docs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoIterationChanges {
    #[serde(default)]
    pub change_entries: Vec<AdoIterationChangeEntry>,
}

/// One entry of `GitPullRequestIterationChanges.changeEntries`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoIterationChangeEntry {
    #[serde(default)]
    pub change_id: i64,
    #[serde(default)]
    pub change_tracking_id: i64,
    #[serde(default)]
    pub item: Option<AdoGitItem>,
    #[serde(default)]
    pub original_path: Option<String>,
    /// The change kind (`"add"`, `"edit"`, `"delete"`, `"rename"`, ...) as
    /// a raw string. Azure DevOps documents this as a flags-style enum
    /// (`VersionControlChangeType`) that can combine values
    /// (e.g. `"rename, edit"`); kept as the raw string rather than a fixed
    /// enum since this adapter only needs it for display, not branching
    /// logic.
    #[serde(default)]
    pub change_type: String,
}

/// `GitItem` — only the path, since content is fetched (when needed at
/// all) via a local checkout rather than the Items API (see
/// `src/forge/azure/backend.rs`'s module doc comment on the diff-text
/// evidence gap).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoGitItem {
    #[serde(default)]
    pub path: String,
}

/// `CommentPosition` — a zero-based line/offset pair used by
/// `CommentThreadContext`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCommentPosition {
    pub line: u32,
    pub offset: u32,
}

/// `CommentThreadContext` — the file/line anchor of a thread. Azure DevOps'
/// anchor model is iteration-relative with independent left (base) and
/// right (head) side start/end offsets that can both be present at once
/// (`SideSupport::simultaneous_both_sides()` /
/// `RangeSupport::DualSideOffsets` in `crate::forge::capabilities`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCommentThreadContext {
    pub file_path: String,
    #[serde(default)]
    pub left_file_start: Option<AdoCommentPosition>,
    #[serde(default)]
    pub left_file_end: Option<AdoCommentPosition>,
    #[serde(default)]
    pub right_file_start: Option<AdoCommentPosition>,
    #[serde(default)]
    pub right_file_end: Option<AdoCommentPosition>,
}

/// `CommentIterationContext` — which iteration-pair a thread's anchor was
/// computed against.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCommentIterationContext {
    pub first_comparing_iteration: u32,
    pub second_comparing_iteration: u32,
}

/// `CommentTrackingCriteria` — the original anchor plus enough context for
/// Azure DevOps' server-side line-tracking to remap a thread onto a later
/// iteration. This is the concrete evidence behind
/// `StaleAnchorSignal::Tracking` in `crate::forge::capabilities`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCommentTrackingCriteria {
    #[serde(default)]
    pub orig_file_path: Option<String>,
    #[serde(default)]
    pub orig_left_file_start: Option<AdoCommentPosition>,
    #[serde(default)]
    pub orig_left_file_end: Option<AdoCommentPosition>,
    #[serde(default)]
    pub orig_right_file_start: Option<AdoCommentPosition>,
    #[serde(default)]
    pub orig_right_file_end: Option<AdoCommentPosition>,
    #[serde(default)]
    pub first_comparing_iteration: Option<u32>,
    #[serde(default)]
    pub second_comparing_iteration: Option<u32>,
}

/// `GitPullRequestCommentThreadContext` — the sibling `pullRequestThreadContext`
/// field alongside `threadContext` on a thread, carrying the
/// change-tracking ID plus the iteration/tracking metadata above.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoPullRequestThreadContext {
    #[serde(default)]
    pub change_tracking_id: Option<i64>,
    #[serde(default)]
    pub iteration_context: Option<AdoCommentIterationContext>,
    #[serde(default)]
    pub tracking_criteria: Option<AdoCommentTrackingCriteria>,
}

/// `CommentThreadStatus`. Evidenced values:
/// `unknown`, `active`, `fixed`, `wontFix`, `closed`, `byDesign`, `pending`.
/// Maps to `ThreadResolutionLevel::Thread` in `crate::forge::capabilities`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AdoThreadStatus {
    #[default]
    Unknown,
    Active,
    Fixed,
    WontFix,
    Closed,
    ByDesign,
    Pending,
}

impl AdoThreadStatus {
    pub fn is_resolved(self) -> bool {
        matches!(
            self,
            AdoThreadStatus::Fixed | AdoThreadStatus::Closed | AdoThreadStatus::ByDesign
        )
    }
}

/// `Comment` — one comment within a thread. `parent_comment_id` is `0` (not
/// absent) for a root comment per Azure DevOps' documented convention, so
/// it is *not* wrapped in `Option` here — callers compare against `0`
/// directly (see `AdoComment::is_reply`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoComment {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub parent_comment_id: i64,
    #[serde(default)]
    pub author: Option<AdoIdentityRef>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub published_date: Option<DateTime<Utc>>,
    #[serde(default)]
    pub is_deleted: bool,
}

impl AdoComment {
    pub fn is_reply(&self) -> bool {
        self.parent_comment_id > 0
    }
}

/// `GitPullRequestCommentThread`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCommentThread {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub status: AdoThreadStatus,
    #[serde(default)]
    pub comments: Vec<AdoComment>,
    #[serde(default)]
    pub thread_context: Option<AdoCommentThreadContext>,
    #[serde(default)]
    pub pull_request_thread_context: Option<AdoPullRequestThreadContext>,
    #[serde(default)]
    pub is_deleted: bool,
}

/// `GET .../threads` list-response envelope (`{ "value": [...], "count": N }`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoListResponse<T> {
    // `default = "Vec::new"` (rather than bare `#[serde(default)]`) avoids
    // serde's derive macro adding an unwanted `T: Default` bound to the
    // generated `Deserialize` impl — `Vec::new` doesn't require `T:
    // Default` even though `Vec::<T>::default()` would trigger the derive
    // macro's conservative per-field bound inference to require it.
    #[serde(default = "Vec::new")]
    pub value: Vec<T>,
}

/// `GitCommitRef` (as returned by the Pull Request Commits endpoint).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCommit {
    #[serde(default)]
    pub commit_id: String,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub author: Option<AdoGitUserDate>,
}

/// `GitUserDate` — `author`/`committer` on a commit.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoGitUserDate {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub date: Option<DateTime<Utc>>,
}

impl AdoCommit {
    pub fn into_pull_request_commit(self) -> PullRequestCommit {
        let short_oid = self.commit_id.chars().take(8).collect();
        let summary = self.comment.lines().next().unwrap_or_default().to_string();
        PullRequestCommit {
            oid: self.commit_id,
            short_oid,
            summary,
            author: self
                .author
                .as_ref()
                .map(|a| a.name.clone())
                .unwrap_or_default(),
            timestamp: self.author.and_then(|a| a.date),
        }
    }
}

// ----- Request bodies -----

/// Body for `PUT .../threads/{id}/comments/{id}` or
/// `POST .../threads/{id}/comments` (reply). Also reused as the sole
/// comment in a new thread's `comments[]` array on thread creation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCreateCommentRequest {
    pub content: String,
}

/// Body for `POST .../threads` (create a new thread — tuicr's mapping of
/// "post an inline/general comment").
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoCreateThreadRequest {
    pub comments: Vec<AdoCreateCommentRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_context: Option<AdoCommentThreadContext>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AdoThreadStatus>,
}

/// Body for `PATCH .../threads/{id}` (status update).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoUpdateThreadStatusRequest {
    pub status: AdoThreadStatus,
}

/// Body for `PUT .../reviewers/{reviewerId}` (cast/update a vote).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoUpdateVoteRequest {
    pub vote: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_map_vote_values_both_ways() {
        for vote in [
            AdoVote::Approved,
            AdoVote::ApprovedWithSuggestions,
            AdoVote::NoVote,
            AdoVote::WaitingForAuthor,
            AdoVote::Rejected,
        ] {
            assert_eq!(AdoVote::from_wire_value(vote.wire_value()), Some(vote));
        }
    }

    #[test]
    fn should_return_none_for_unrecognized_vote_value() {
        assert_eq!(AdoVote::from_wire_value(7), None);
    }

    #[test]
    fn should_serialize_thread_status_as_lower_camel_case() {
        assert_eq!(
            serde_json::to_string(&AdoThreadStatus::WontFix).unwrap(),
            "\"wontFix\""
        );
        assert_eq!(
            serde_json::to_string(&AdoThreadStatus::ByDesign).unwrap(),
            "\"byDesign\""
        );
        assert_eq!(
            serde_json::to_string(&AdoThreadStatus::Active).unwrap(),
            "\"active\""
        );
    }

    #[test]
    fn should_classify_resolved_thread_statuses() {
        assert!(AdoThreadStatus::Fixed.is_resolved());
        assert!(AdoThreadStatus::Closed.is_resolved());
        assert!(AdoThreadStatus::ByDesign.is_resolved());
        assert!(!AdoThreadStatus::Active.is_resolved());
        assert!(!AdoThreadStatus::Pending.is_resolved());
        assert!(!AdoThreadStatus::Unknown.is_resolved());
    }

    #[test]
    fn should_treat_zero_parent_comment_id_as_root() {
        let root = AdoComment {
            id: 1,
            parent_comment_id: 0,
            author: None,
            content: "root".to_string(),
            published_date: None,
            is_deleted: false,
        };
        let reply = AdoComment {
            parent_comment_id: 1,
            ..root.clone()
        };
        assert!(!root.is_reply());
        assert!(reply.is_reply());
    }

    #[test]
    fn should_strip_refs_heads_prefix() {
        assert_eq!(strip_ref_prefix("refs/heads/main"), "main");
        assert_eq!(strip_ref_prefix("main"), "main");
    }
}
