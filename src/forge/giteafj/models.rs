//! Wire models for the Gitea/Forgejo REST API surface.
//!
//! Field names and JSON tags are taken verbatim from the upstream Go
//! `modules/structs` package (both Gitea `main` and the Forgejo `forgejo`
//! branch share the same tags for every type here except
//! `CreatePullReviewComment`/`PullReviewComment`, where Forgejo additionally
//! declares `extra_lines_count`; Gitea's struct omits that field entirely,
//! which is why it is modeled as `#[serde(default)] Option<i64>` here rather
//! than a plain `i64` — Gitea responses simply won't have the key).
//!
//! See:
//! - `https://github.com/go-gitea/gitea/blob/main/modules/structs/pull.go`
//! - `https://codeberg.org/forgejo/forgejo/src/branch/forgejo/modules/structs/pull_review.go`
//! - `https://github.com/go-gitea/gitea/blob/main/modules/structs/repo_commit.go`
//! - `https://github.com/go-gitea/gitea/blob/main/modules/structs/user.go`
//! - `https://github.com/go-gitea/gitea/blob/main/modules/structs/miscellaneous.go`
//!   (`ServerVersion{Version string \`json:"version"\`}`)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{RemoteReviewState, RemoteReviewSummary};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestSummary,
};

/// Some Gitea/Forgejo list endpoints serialize an empty array-typed field as
/// JSON `null` rather than `[]` (observed live on Gitea 1.24's
/// `requested_reviewers`). `#[serde(default)]` alone only covers a *missing*
/// key, not a present `null` value, so plain `Vec<T>` fields need this
/// deserializer to tolerate both.
fn null_to_default_vec<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
pub struct GfVersionResponse {
    pub version: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GfUser {
    #[serde(default)]
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub struct GfBranchInfo {
    #[serde(rename = "ref")]
    #[serde(default)]
    pub git_ref: String,
    #[serde(default)]
    pub sha: String,
}

#[derive(Debug, Deserialize)]
pub struct GfPullRequest {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub user: Option<GfUser>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
    pub base: GfBranchInfo,
    pub head: GfBranchInfo,
    /// Present only on list responses when the viewer has been requested as
    /// a reviewer — used client-side to emulate the `ReviewRequested` scope
    /// filter, since neither Gitea nor Forgejo exposes a server-side
    /// `review-requested:@me` search filter the way GitHub/GitLab do.
    #[serde(default, deserialize_with = "null_to_default_vec")]
    pub requested_reviewers: Vec<GfUser>,
}

impl GfPullRequest {
    pub fn into_summary(self, repository: &ForgeRepository) -> PullRequestSummary {
        PullRequestSummary {
            repository: repository.clone(),
            number: self.number,
            title: self.title,
            author: self.user.map(|u| u.login),
            head_ref_name: self.head.git_ref,
            base_ref_name: self.base.git_ref,
            updated_at: self.updated_at,
            url: self.html_url,
            state: self.state,
            is_draft: self.draft,
        }
    }

    pub fn into_details(self, repository: &ForgeRepository) -> Result<PullRequestDetails> {
        require_field(&self.head.sha, "head.sha")?;
        require_field(&self.base.sha, "base.sha")?;
        Ok(PullRequestDetails {
            repository: repository.clone(),
            number: self.number,
            title: self.title,
            url: self.html_url,
            state: self.state,
            is_draft: self.draft,
            author: self.user.map(|u| u.login),
            head_ref_name: self.head.git_ref,
            base_ref_name: self.base.git_ref,
            head_sha: self.head.sha,
            base_sha: self.base.sha,
            body: self.body,
            updated_at: self.updated_at,
            closed: self.closed_at.is_some(),
            merged_at: if self.merged { self.merged_at } else { None },
            diff_start_sha: None,
        })
    }

    /// Whether `viewer_login` appears among this PR's requested reviewers —
    /// the client-side substitute for a server-side `ReviewRequested` scope
    /// filter (see `requested_reviewers` field doc).
    pub fn requests_review_from(&self, viewer_login: &str) -> bool {
        self.requested_reviewers
            .iter()
            .any(|u| u.login == viewer_login)
    }
}

fn require_field(value: &str, field: &str) -> Result<()> {
    if value.is_empty() {
        Err(TuicrError::Forge(format!(
            "Gitea/Forgejo response did not include required field `{field}`"
        )))
    } else {
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct GfCommitMeta {
    #[serde(default)]
    pub sha: String,
}

#[derive(Debug, Deserialize)]
pub struct GfCommitUser {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub email: String,
    /// Gitea/Forgejo return this as a string, not a typed timestamp (see
    /// `CommitUser.Date string \`json:"date"\`` upstream) — parsed
    /// best-effort via `DateTime::parse_from_rfc3339`.
    #[serde(default)]
    pub date: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct GfRepoCommit {
    #[serde(default)]
    pub author: Option<GfCommitUser>,
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct GfCommit {
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub commit: GfRepoCommit,
}

impl GfCommit {
    pub fn into_pull_request_commit(self) -> PullRequestCommit {
        let summary = self.commit.message.lines().next().unwrap_or("").to_string();
        let author = self
            .commit
            .author
            .as_ref()
            .map(|a| {
                if !a.name.is_empty() {
                    a.name.clone()
                } else if !a.email.is_empty() {
                    a.email.clone()
                } else {
                    "unknown".to_string()
                }
            })
            .unwrap_or_else(|| "unknown".to_string());
        let timestamp = self
            .commit
            .author
            .as_ref()
            .and_then(|a| DateTime::parse_from_rfc3339(&a.date).ok())
            .map(|dt| dt.with_timezone(&Utc));
        let short_oid = self.sha.chars().take(7).collect();
        PullRequestCommit {
            oid: self.sha,
            short_oid,
            summary,
            author,
            timestamp,
        }
    }
}

/// `PullReview` — see `pull_review.go`. `state` uses the provider's own
/// `ReviewStateType` literals (`APPROVED`/`PENDING`/`COMMENT`/
/// `REQUEST_CHANGES`/`REQUEST_REVIEW`), which do **not** match
/// `RemoteReviewState::parse`'s GitHub-shaped vocabulary (notably
/// `REQUEST_CHANGES` vs GitHub's `CHANGES_REQUESTED`) — callers must use
/// `GfReview::remote_state`, not the generic parser, for this family.
#[derive(Debug, Clone, Deserialize)]
pub struct GfReview {
    pub id: u64,
    #[serde(default)]
    pub user: Option<GfUser>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub commit_id: String,
    #[serde(default)]
    pub stale: bool,
    #[serde(default)]
    pub submitted_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub html_url: String,
}

impl GfReview {
    /// Map this family's `ReviewStateType` literal onto the shared
    /// `RemoteReviewState` vocabulary. Deliberately not
    /// `RemoteReviewState::parse` (see struct doc) — `REQUEST_CHANGES`
    /// would silently fall through that parser's default `Commented` arm.
    pub fn remote_state(&self) -> RemoteReviewState {
        match self.state.as_str() {
            "APPROVED" => RemoteReviewState::Approved,
            "REQUEST_CHANGES" => RemoteReviewState::ChangesRequested,
            "PENDING" => RemoteReviewState::Pending,
            _ => RemoteReviewState::Commented,
        }
    }

    pub fn into_summary(self) -> Option<RemoteReviewSummary> {
        if self.body.trim().is_empty() {
            return None;
        }
        let state = self.remote_state();
        Some(RemoteReviewSummary {
            id: self.id.to_string(),
            author: self.user.map(|u| u.login),
            body: self.body,
            state,
            created_at: self.submitted_at,
            url: self.html_url,
        })
    }
}

/// `PullReviewComment` — see `pull_review.go`. `extra_lines_count` is
/// `Option` because Gitea's struct does not declare the field at all (it is
/// a Forgejo-only extension); Gitea responses simply omit the key, which
/// `#[serde(default)]` turns into `None` rather than a deserialization
/// error.
#[derive(Debug, Clone, Deserialize)]
pub struct GfReviewComment {
    pub id: u64,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub user: Option<GfUser>,
    #[serde(default)]
    pub resolver: Option<GfUser>,
    #[serde(default)]
    pub pull_request_review_id: u64,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub path: String,
    /// New-side (`position`, JSON `position`) line number, `0` when the
    /// comment anchors only to the old side.
    #[serde(default)]
    pub position: u64,
    /// Old-side (`original_position`) line number, `0` when the comment
    /// anchors only to the new side.
    #[serde(default)]
    pub original_position: u64,
    #[serde(default)]
    pub extra_lines_count: Option<i64>,
    #[serde(default)]
    pub html_url: String,
}

/// Request body for `POST .../pulls/{index}/reviews` (`CreatePullReviewOptions`).
#[derive(Debug, Serialize)]
pub struct GfCreateReviewRequest {
    pub event: &'static str,
    pub body: String,
    pub commit_id: String,
    pub comments: Vec<GfCreateReviewComment>,
}

/// `CreatePullReviewComment`/`CreatePullReviewCommentOptions` — same shape
/// used both in the review-creation `comments[]` array and (Forgejo only)
/// the single-object add-to-existing-review endpoint.
#[derive(Debug, Serialize)]
pub struct GfCreateReviewComment {
    pub path: String,
    pub body: String,
    pub old_position: i64,
    pub new_position: i64,
    /// Omitted entirely (not sent as `0`) for single-line comments and for
    /// Gitea, which does not declare this field — sending it is harmless
    /// there (extra JSON keys are ignored) but a range value would
    /// misleadingly imply the comment could be a native range when Gitea
    /// silently drops it; the backend never sets this for Gitea (see
    /// `RangeSupport::None` handling in `backend.rs`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra_lines_count: Option<i64>,
}

/// Response shape for `POST .../pulls/{index}/reviews` — a `PullReview`.
#[derive(Debug, Deserialize)]
pub struct GfCreateReviewResponse {
    pub id: u64,
    #[serde(default)]
    pub html_url: String,
    #[serde(default)]
    pub state: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> ForgeRepository {
        ForgeRepository::gitea("http://127.0.0.1:1234", "owner", "repo")
    }

    #[test]
    fn should_deserialize_minimal_pull_request_json() {
        let json = r#"{
            "number": 7,
            "title": "Add feature",
            "body": "desc",
            "user": {"login": "alice"},
            "state": "open",
            "draft": false,
            "html_url": "http://x/pulls/7",
            "merged": false,
            "base": {"ref": "main", "sha": "base123"},
            "head": {"ref": "feature", "sha": "head456"}
        }"#;
        let pr: GfPullRequest = serde_json::from_str(json).expect("parse");
        let details = pr.into_details(&repo()).expect("into_details");
        assert_eq!(details.number, 7);
        assert_eq!(details.head_sha, "head456");
        assert_eq!(details.base_sha, "base123");
        assert_eq!(details.author.as_deref(), Some("alice"));
        assert!(!details.closed);
        assert!(details.merged_at.is_none());
    }

    #[test]
    fn should_treat_closed_at_present_as_closed_regardless_of_merged() {
        let json = r#"{
            "number": 1, "title": "t", "body": "", "state": "closed", "draft": false,
            "html_url": "u", "merged": false, "closed_at": "2024-01-01T00:00:00Z",
            "base": {"ref": "main", "sha": "b"}, "head": {"ref": "h", "sha": "h"}
        }"#;
        let pr: GfPullRequest = serde_json::from_str(json).expect("parse");
        let details = pr.into_details(&repo()).expect("into_details");
        assert!(details.closed);
        assert!(details.merged_at.is_none());
    }

    #[test]
    fn should_error_when_head_sha_missing() {
        let json = r#"{
            "number": 1, "title": "t", "body": "", "state": "open", "draft": false,
            "html_url": "u", "merged": false,
            "base": {"ref": "main", "sha": "b"}, "head": {"ref": "h", "sha": ""}
        }"#;
        let pr: GfPullRequest = serde_json::from_str(json).expect("parse");
        assert!(pr.into_details(&repo()).is_err());
    }

    #[test]
    fn should_detect_requested_reviewer_match() {
        let json = r#"{
            "number": 1, "title": "t", "body": "", "state": "open", "draft": false,
            "html_url": "u", "merged": false,
            "base": {"ref": "main", "sha": "b"}, "head": {"ref": "h", "sha": "h"},
            "requested_reviewers": [{"login": "bob"}, {"login": "carol"}]
        }"#;
        let pr: GfPullRequest = serde_json::from_str(json).expect("parse");
        assert!(pr.requests_review_from("bob"));
        assert!(!pr.requests_review_from("dave"));
    }

    #[test]
    fn should_treat_null_requested_reviewers_as_empty_not_a_parse_error() {
        // Observed live on Gitea 1.24: an empty `requested_reviewers` comes
        // back as JSON `null`, not `[]`. `#[serde(default)]` alone only
        // covers a missing key, so this must use the null-tolerant
        // deserializer or every list_pull_requests call on a PR with no
        // requested reviewers would fail to parse.
        let json = r#"{
            "number": 1, "title": "t", "body": "", "state": "open", "draft": false,
            "html_url": "u", "merged": false,
            "base": {"ref": "main", "sha": "b"}, "head": {"ref": "h", "sha": "h"},
            "requested_reviewers": null
        }"#;
        let pr: GfPullRequest = serde_json::from_str(json).expect("parse");
        assert!(pr.requested_reviewers.is_empty());
        assert!(!pr.requests_review_from("anyone"));
    }

    #[test]
    fn should_parse_review_comment_without_extra_lines_count_field_as_gitea_omits_it() {
        let json = r#"{
            "id": 5, "body": "nit", "user": {"login": "alice"},
            "pull_request_review_id": 2, "path": "src/lib.rs",
            "position": 10, "original_position": 0
        }"#;
        let comment: GfReviewComment = serde_json::from_str(json).expect("parse");
        assert_eq!(comment.extra_lines_count, None);
        assert_eq!(comment.position, 10);
    }

    #[test]
    fn should_parse_review_comment_with_extra_lines_count_when_forgejo_includes_it() {
        let json = r#"{
            "id": 5, "body": "nit", "position": 10, "original_position": 0,
            "extra_lines_count": 3
        }"#;
        let comment: GfReviewComment = serde_json::from_str(json).expect("parse");
        assert_eq!(comment.extra_lines_count, Some(3));
    }

    #[test]
    fn should_map_request_changes_state_distinctly_from_generic_parser() {
        let review = GfReview {
            id: 1,
            user: None,
            state: "REQUEST_CHANGES".to_string(),
            body: "please fix".to_string(),
            commit_id: "sha".to_string(),
            stale: false,
            submitted_at: None,
            html_url: "u".to_string(),
        };
        assert_eq!(review.remote_state(), RemoteReviewState::ChangesRequested);
        // Sanity: the shared GitHub-shaped parser would NOT get this right,
        // which is exactly why `GfReview::remote_state` exists instead.
        assert_ne!(
            RemoteReviewState::parse(&review.state),
            RemoteReviewState::ChangesRequested
        );
    }

    #[test]
    fn should_drop_empty_body_reviews_from_summaries() {
        let review = GfReview {
            id: 1,
            user: Some(GfUser {
                login: "alice".to_string(),
            }),
            state: "APPROVED".to_string(),
            body: "   ".to_string(),
            commit_id: "sha".to_string(),
            stale: false,
            submitted_at: None,
            html_url: "u".to_string(),
        };
        assert!(review.into_summary().is_none());
    }

    #[test]
    fn should_skip_serializing_extra_lines_count_when_none() {
        let comment = GfCreateReviewComment {
            path: "a.rs".to_string(),
            body: "b".to_string(),
            old_position: 0,
            new_position: 5,
            extra_lines_count: None,
        };
        let json = serde_json::to_string(&comment).expect("serialize");
        assert!(!json.contains("extra_lines_count"));
    }

    #[test]
    fn should_include_extra_lines_count_when_some() {
        let comment = GfCreateReviewComment {
            path: "a.rs".to_string(),
            body: "b".to_string(),
            old_position: 0,
            new_position: 5,
            extra_lines_count: Some(4),
        };
        let json = serde_json::to_string(&comment).expect("serialize");
        assert!(json.contains("\"extra_lines_count\":4"));
    }

    #[test]
    fn should_convert_commit_json_into_pull_request_commit() {
        let json = r#"{
            "sha": "abcdef1234567890",
            "commit": {
                "message": "Fix bug\n\nLonger body",
                "author": {"name": "Alice", "email": "a@example.com", "date": "2024-01-02T03:04:05Z"}
            }
        }"#;
        let commit: GfCommit = serde_json::from_str(json).expect("parse");
        let pr_commit = commit.into_pull_request_commit();
        assert_eq!(pr_commit.oid, "abcdef1234567890");
        assert_eq!(pr_commit.short_oid, "abcdef1");
        assert_eq!(pr_commit.summary, "Fix bug");
        assert_eq!(pr_commit.author, "Alice");
        assert!(pr_commit.timestamp.is_some());
    }
}
