use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};

use crate::error::{Result, TuicrError};
use crate::forge::remote_comments::{
    RemoteCommentSide, RemoteReviewComment, RemoteReviewRange, RemoteReviewThread,
};
use crate::forge::traits::{
    ForgeRepository, PullRequestCommit, PullRequestDetails, PullRequestSummary,
};

#[derive(Debug, Deserialize)]
pub struct GlabMrSummary {
    pub iid: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author: Option<GlabUser>,
    #[serde(default)]
    pub source_branch: String,
    #[serde(default)]
    pub target_branch: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
}

impl GlabMrSummary {
    pub fn into_summary(self, repo: &ForgeRepository) -> PullRequestSummary {
        PullRequestSummary {
            repository: repo.clone(),
            number: self.iid,
            title: self.title,
            author: self.author.map(|a| a.username),
            head_ref_name: self.source_branch,
            base_ref_name: self.target_branch,
            updated_at: self.updated_at,
            url: self.web_url,
            state: normalize_state(&self.state),
            is_draft: self.draft,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GlabMrDetails {
    pub iid: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub web_url: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub author: Option<GlabUser>,
    #[serde(default)]
    pub source_branch: String,
    #[serde(default)]
    pub target_branch: String,
    /// Head SHA — last commit on the source branch.
    #[serde(default)]
    pub sha: String,
    #[serde(default)]
    pub diff_refs: Option<GlabDiffRefs>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub closed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub merged_at: Option<DateTime<Utc>>,
}

impl GlabMrDetails {
    pub fn into_details(self, repo: &ForgeRepository) -> Result<PullRequestDetails> {
        let (head_sha, base_sha, diff_start_sha) = match self.diff_refs {
            Some(refs) => (refs.head_sha, refs.base_sha, Some(refs.start_sha)),
            None => {
                // Fall back to `sha` for head; base is unknown
                if self.sha.is_empty() {
                    return Err(TuicrError::Forge(
                        "GitLab MR response missing diff_refs and sha".to_string(),
                    ));
                }
                (self.sha.clone(), String::new(), None)
            }
        };
        let closed = self.state == "closed";
        let state = normalize_state(&self.state);
        Ok(PullRequestDetails {
            repository: repo.clone(),
            number: self.iid,
            title: self.title,
            url: self.web_url,
            state,
            is_draft: self.draft,
            author: self.author.map(|a| a.username),
            head_ref_name: self.source_branch,
            base_ref_name: self.target_branch,
            head_sha,
            base_sha,
            body: self.description,
            updated_at: self.updated_at,
            closed,
            merged_at: self.merged_at,
            diff_start_sha,
        })
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabDiffRefs {
    #[serde(default)]
    pub base_sha: String,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub start_sha: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabUser {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct GlabMrVersion {
    #[serde(default)]
    pub head_commit_sha: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabApprovalState {
    #[serde(default)]
    pub approved_by: Vec<GlabApprovedBy>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabApprovedBy {
    #[serde(default)]
    pub user: GlabUser,
    #[serde(default)]
    pub approved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct GlabCommit {
    pub id: String,
    #[serde(default)]
    pub short_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub author_name: String,
    #[serde(default)]
    pub committed_date: Option<DateTime<Utc>>,
}

impl GlabCommit {
    pub fn into_pull_request_commit(self) -> PullRequestCommit {
        let short_oid = if self.short_id.is_empty() {
            self.id.chars().take(7).collect()
        } else {
            self.short_id.clone()
        };
        PullRequestCommit {
            oid: self.id,
            short_oid,
            summary: self.title,
            author: self.author_name,
            timestamp: self.committed_date,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GlabDiscussion {
    pub id: String,
    #[serde(default)]
    pub individual_note: bool,
    #[serde(default)]
    pub notes: Vec<GlabNote>,
}

/// Convert `notes` (already filtered/ordered so `notes[0]` is the thread
/// root) into [`RemoteReviewComment`]s, populating `in_reply_to` with
/// thread-grouping semantics: GitLab discussions are flat (every note in a
/// discussion belongs to the same thread; there is no nested reply-to-a
/// -specific-reply structure), so every non-root note's `in_reply_to`
/// points at the root note's own ID — mirroring how GitHub's GraphQL
/// `replyTo` also always resolves to the thread's original comment even
/// for a reply to a reply (see `github/review_threads.rs`'s
/// `should_parse_multi_comment_thread_with_replies`, where the second and
/// third comments both carry `in_reply_to: Some("PRRC_1")`). Root/reply
/// *order* was already preserved by relying on `notes`' own array order
/// (GitLab returns notes in creation order); this only fixes the
/// previously-hardcoded `in_reply_to: None` on every note.
fn notes_into_comments(notes: Vec<GlabNote>) -> Vec<RemoteReviewComment> {
    let root_id = notes.first().map(|n| n.id.to_string());
    notes
        .into_iter()
        .enumerate()
        .map(|(index, note)| {
            let in_reply_to = if index == 0 { None } else { root_id.clone() };
            RemoteReviewComment {
                id: note.id.to_string(),
                author: Some(note.author.username),
                body: note.body,
                created_at: note.created_at,
                in_reply_to,
                url: String::new(),
                // GitLab discussion/note `id`s are already REST-compatible
                // (`reply_to_thread` reads the bare mapping's `id`
                // directly, never `root_comment_id`), so there is nothing
                // distinct to capture here.
                rest_id: None,
            }
        })
        .collect()
}

impl GlabDiscussion {
    pub fn into_review_thread(self, current_mr_head: Option<&str>) -> Option<RemoteReviewThread> {
        let root = self.notes.first()?;

        if self.individual_note {
            // General MR note (review summary), no diff position.
            // Skip system events (e.g. "requested review from @X") and empty bodies.
            if root.system || root.body.is_empty() {
                return None;
            }
            let notes: Vec<GlabNote> = self
                .notes
                .into_iter()
                .filter(|n| !n.system && !n.body.is_empty())
                .collect();
            if notes.is_empty() {
                return None;
            }
            let comments = notes_into_comments(notes);
            return Some(RemoteReviewThread {
                id: self.id,
                path: String::new(),
                line: None,
                side: RemoteCommentSide::Right,
                is_resolved: false,
                is_outdated: false,
                range: None,
                provider_native_anchor: None,
                comments,
            });
        }

        // Only inline (positional) discussions have a position on the root note.
        let position = root.position.as_ref()?;

        // Skip non-text positions (e.g. image diffs).
        if position.position_type != "text" {
            return None;
        }

        let (path, mut line, side) = if let Some(new_line) = position.new_line {
            // Comment on the new (right) side.
            let path = position.new_path.clone().unwrap_or_default();
            (path, Some(new_line), RemoteCommentSide::Right)
        } else {
            let old_line = position.old_line?;
            // Comment on the old (left) side only.
            let path = position
                .old_path
                .clone()
                .or_else(|| position.new_path.clone())
                .unwrap_or_default();
            (path, Some(old_line), RemoteCommentSide::Left)
        };

        if path.is_empty() {
            return None;
        }

        let is_resolved = root.resolved;
        let provider_native_anchor = Some(position.raw().clone());
        let head_mismatch = version_mismatch(position.head_sha.as_deref(), current_mr_head);
        let (range, invalid_range) = validated_range(position, side, line, !head_mismatch);
        if head_mismatch || invalid_range {
            line = range.as_ref().map(|range| range.end()).or(line);
        }
        let is_outdated = invalid_range || head_mismatch;
        let comments = notes_into_comments(self.notes);

        Some(RemoteReviewThread {
            id: self.id,
            path,
            line,
            side,
            is_resolved,
            is_outdated,
            range,
            provider_native_anchor,
            comments,
        })
    }
}

fn version_mismatch(position_head: Option<&str>, current_mr_head: Option<&str>) -> bool {
    match (
        position_head.filter(|value| !value.is_empty()),
        current_mr_head.filter(|value| !value.is_empty()),
    ) {
        (Some(position_head), Some(current_mr_head)) => position_head != current_mr_head,
        _ => false,
    }
}

fn validated_range(
    position: &GlabNotePosition,
    side: RemoteCommentSide,
    terminal_line: Option<u32>,
    require_terminal_match: bool,
) -> (Option<RemoteReviewRange>, bool) {
    let Some(line_range) = position.line_range.as_ref() else {
        return (None, false);
    };
    let Some(terminal_line) = terminal_line else {
        return (None, true);
    };
    let Some(start) = line_range.start.line_for_side(side) else {
        return (None, true);
    };
    let Some(end) = line_range.end.line_for_side(side) else {
        return (None, true);
    };
    match RemoteReviewRange::new(start, end) {
        Ok(range) => (Some(range), require_terminal_match && end != terminal_line),
        Err(_) => (None, true),
    }
}

#[derive(Debug, Deserialize)]
pub struct GlabNote {
    pub id: u64,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub author: GlabNoteAuthor,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub commit_id: Option<String>,
    #[serde(default)]
    pub position: Option<GlabNotePosition>,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub system: bool,
}

#[derive(Debug, Deserialize, Default)]
pub struct GlabNoteAuthor {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug)]
pub struct GlabNotePosition {
    raw: serde_json::Value,
    pub position_type: String,
    pub head_sha: Option<String>,
    pub new_path: Option<String>,
    pub new_line: Option<u32>,
    pub old_path: Option<String>,
    pub old_line: Option<u32>,
    pub line_range: Option<GlabNoteLineRange>,
}

impl GlabNotePosition {
    fn raw(&self) -> &serde_json::Value {
        &self.raw
    }
}

#[derive(Debug, Deserialize)]
struct GlabNotePositionFields {
    #[serde(default)]
    position_type: String,
    #[serde(default)]
    head_sha: Option<String>,
    new_path: Option<String>,
    new_line: Option<u32>,
    old_path: Option<String>,
    old_line: Option<u32>,
    #[serde(default)]
    line_range: Option<GlabNoteLineRange>,
}

impl<'de> Deserialize<'de> for GlabNotePosition {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = serde_json::Value::deserialize(deserializer)?;
        let fields: GlabNotePositionFields =
            serde_json::from_value(raw.clone()).map_err(serde::de::Error::custom)?;
        Ok(Self {
            raw,
            position_type: fields.position_type,
            head_sha: fields.head_sha,
            new_path: fields.new_path,
            new_line: fields.new_line,
            old_path: fields.old_path,
            old_line: fields.old_line,
            line_range: fields.line_range,
        })
    }
}

#[derive(Debug, Deserialize)]
pub struct GlabNoteLineRange {
    pub start: GlabNoteLineRangeEndpoint,
    pub end: GlabNoteLineRangeEndpoint,
}

#[derive(Debug, Deserialize)]
pub struct GlabNoteLineRangeEndpoint {
    #[serde(default, rename = "type")]
    pub line_type: Option<String>,
    #[serde(default)]
    pub old_line: Option<u32>,
    #[serde(default)]
    pub new_line: Option<u32>,
    #[serde(default)]
    pub line_code: Option<String>,
}

impl GlabNoteLineRangeEndpoint {
    fn line_for_side(&self, side: RemoteCommentSide) -> Option<u32> {
        match (self.line_type.as_deref(), side) {
            (Some("new"), RemoteCommentSide::Right) => self.new_line,
            (Some("old"), RemoteCommentSide::Left) => self.old_line,
            (None, RemoteCommentSide::Right)
                if self.old_line.is_some() && self.new_line.is_some() =>
            {
                self.new_line
            }
            (None, RemoteCommentSide::Left)
                if self.old_line.is_some() && self.new_line.is_some() =>
            {
                self.old_line
            }
            _ => None,
        }
    }
}

fn normalize_state(state: &str) -> String {
    match state.to_ascii_lowercase().as_str() {
        "opened" | "open" => "OPEN".to_string(),
        "merged" => "MERGED".to_string(),
        "closed" => "CLOSED".to_string(),
        other => other.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    fn gitlab_repo() -> ForgeRepository {
        ForgeRepository::gitlab("gitlab.com", "owner", "repo")
    }

    fn position(value: serde_json::Value) -> GlabNotePosition {
        serde_json::from_value(value).unwrap()
    }

    fn positional_discussion(id: &str, position: serde_json::Value) -> GlabDiscussion {
        GlabDiscussion {
            id: id.to_string(),
            individual_note: false,
            notes: vec![GlabNote {
                id: 100,
                body: "review comment".to_string(),
                author: GlabNoteAuthor {
                    username: "bob".to_string(),
                    name: "Bob".to_string(),
                },
                created_at: None,
                commit_id: None,
                position: Some(self::position(position)),
                resolved: false,
                system: false,
            }],
        }
    }

    #[test]
    fn should_deserialize_glab_mr_summary() {
        let json = r#"{
            "iid": 42,
            "title": "My MR",
            "author": { "username": "alice", "name": "Alice" },
            "source_branch": "feature",
            "target_branch": "main",
            "updated_at": "2024-01-01T00:00:00Z",
            "web_url": "https://gitlab.com/owner/repo/-/merge_requests/42",
            "state": "opened",
            "draft": false
        }"#;
        let summary: GlabMrSummary = serde_json::from_str(json).unwrap();
        assert_eq!(summary.iid, 42);
        assert_eq!(summary.title, "My MR");
        assert_eq!(summary.author.as_ref().unwrap().username, "alice");
        assert_eq!(summary.source_branch, "feature");
        assert_eq!(summary.target_branch, "main");
        let pr_summary = summary.into_summary(&gitlab_repo());
        assert_eq!(pr_summary.number, 42);
        assert_eq!(pr_summary.state, "OPEN");
    }

    #[test]
    fn should_deserialize_glab_mr_details_with_diff_refs() {
        let json = r#"{
            "iid": 42,
            "title": "My MR",
            "web_url": "https://gitlab.com/owner/repo/-/merge_requests/42",
            "state": "opened",
            "draft": false,
            "author": { "username": "alice", "name": "Alice" },
            "source_branch": "feature",
            "target_branch": "main",
            "sha": "head111",
            "diff_refs": {
                "base_sha": "base000",
                "head_sha": "head111",
                "start_sha": "start222"
            },
            "description": "desc",
            "merged_at": null,
            "closed_at": null
        }"#;
        let details: GlabMrDetails = serde_json::from_str(json).unwrap();
        assert_eq!(details.iid, 42);
        let pr = details.into_details(&gitlab_repo()).unwrap();
        assert_eq!(pr.head_sha, "head111");
        assert_eq!(pr.base_sha, "base000");
        assert_eq!(pr.diff_start_sha, Some("start222".to_string()));
        assert!(!pr.closed);
    }

    #[test]
    fn should_convert_glab_discussion_to_review_thread() {
        let discussion = GlabDiscussion {
            id: "disc-1".to_string(),
            individual_note: false,
            notes: vec![GlabNote {
                id: 100,
                body: "review comment".to_string(),
                author: GlabNoteAuthor {
                    username: "bob".to_string(),
                    name: "Bob".to_string(),
                },
                created_at: None,
                commit_id: None,
                position: Some(position(serde_json::json!({
                    "position_type": "text",
                    "new_path": "src/lib.rs",
                    "new_line": 42
                }))),
                resolved: false,
                system: false,
            }],
        };
        let thread = discussion.into_review_thread(None).unwrap();
        assert_eq!(thread.id, "disc-1");
        assert_eq!(thread.path, "src/lib.rs");
        assert_eq!(thread.line, Some(42));
        assert_eq!(thread.side, RemoteCommentSide::Right);
        assert!(!thread.is_resolved);
        assert!(!thread.is_outdated);
        assert!(thread.range.is_none());
        assert_eq!(
            thread.provider_native_anchor.unwrap()["new_line"],
            serde_json::json!(42)
        );
        assert_eq!(thread.comments[0].author.as_deref(), Some("bob"));
    }

    #[test]
    fn should_classify_current_and_old_head_ranges_and_preserve_native_anchor() {
        let position = |head_sha: &str, terminal_line: u32| {
            serde_json::json!({
                "position_type": "text",
                "base_sha": "base-sha",
                "start_sha": "start-sha",
                "head_sha": head_sha,
                "new_path": "src/lib.rs",
                "new_line": terminal_line,
                "line_range": {
                    "start": {
                        "line_code": "start-code",
                        "type": "new",
                        "new_line": 10
                    },
                    "end": {
                        "line_code": "end-code",
                        "type": "new",
                        "new_line": 12
                    }
                },
                "unmodeled_provider_field": "preserved"
            })
        };

        let current = positional_discussion("current", position("head-current", 12))
            .into_review_thread(Some("head-current"))
            .unwrap();
        assert!(!current.is_outdated);
        assert_eq!(current.range.as_ref().unwrap().start(), 10);
        assert_eq!(current.range.as_ref().unwrap().end(), 12);
        assert_eq!(
            current.provider_native_anchor.as_ref().unwrap()["unmodeled_provider_field"],
            serde_json::json!("preserved")
        );

        let outdated = positional_discussion("outdated", position("head-old", 17))
            .into_review_thread(Some("head-current"))
            .unwrap();
        assert!(outdated.is_outdated);
        assert_eq!(outdated.line, Some(12));
        assert_eq!(outdated.range.as_ref().unwrap().start(), 10);
        assert_eq!(outdated.range.as_ref().unwrap().end(), 12);
        assert_eq!(
            outdated.provider_native_anchor.as_ref().unwrap()["new_line"],
            serde_json::json!(17)
        );
    }

    #[test]
    fn should_accept_context_line_range_endpoints_for_the_selected_side() {
        let discussion = positional_discussion(
            "context-range",
            serde_json::json!({
                "position_type": "text",
                "head_sha": "head-current",
                "new_path": "src/lib.rs",
                "new_line": 12,
                "line_range": {
                    "start": {
                        "line_code": "context-code",
                        "type": null,
                        "old_line": 9,
                        "new_line": 10
                    },
                    "end": {
                        "line_code": "new-code",
                        "type": "new",
                        "new_line": 12
                    }
                }
            }),
        );

        let thread = discussion.into_review_thread(Some("head-current")).unwrap();
        assert!(!thread.is_outdated);
        assert_eq!(thread.range.as_ref().unwrap().start(), 10);
        assert_eq!(thread.range.as_ref().unwrap().end(), 12);
    }

    #[test]
    fn should_mark_malformed_ranges_outdated_without_collapsing_or_losing_native_data() {
        let malformed_ranges = [
            serde_json::json!({
                "start": {"type": "old", "old_line": 10},
                "end": {"type": "new", "new_line": 12}
            }),
            serde_json::json!({
                "start": {"type": "new", "new_line": 13},
                "end": {"type": "new", "new_line": 12}
            }),
            serde_json::json!({
                "start": {"type": "unknown", "new_line": 10},
                "end": {"type": "new", "new_line": 12}
            }),
        ];

        for (index, line_range) in malformed_ranges.into_iter().enumerate() {
            let discussion = positional_discussion(
                &format!("malformed-{index}"),
                serde_json::json!({
                    "position_type": "text",
                    "head_sha": "head-current",
                    "new_path": "src/lib.rs",
                    "new_line": 12,
                    "line_range": line_range
                }),
            );
            let thread = discussion.into_review_thread(Some("head-current")).unwrap();
            assert!(thread.is_outdated, "malformed case {index}");
            assert!(thread.range.is_none(), "malformed case {index}");
            assert!(
                thread.provider_native_anchor.as_ref().unwrap()["line_range"].is_object(),
                "malformed case {index}"
            );
        }
    }

    #[test]
    fn should_preserve_terminal_mismatched_range_as_outdated() {
        let discussion = positional_discussion(
            "terminal-mismatch",
            serde_json::json!({
                "position_type": "text",
                "head_sha": "head-current",
                "new_path": "src/lib.rs",
                "new_line": 17,
                "line_range": {
                    "start": {"type": "new", "new_line": 10},
                    "end": {"type": "new", "new_line": 12}
                }
            }),
        );

        let thread = discussion.into_review_thread(Some("head-current")).unwrap();
        assert!(thread.is_outdated);
        assert_eq!(thread.line, Some(12));
        assert_eq!(thread.range.as_ref().unwrap().start(), 10);
        assert_eq!(thread.range.as_ref().unwrap().end(), 12);
        assert_eq!(
            thread.provider_native_anchor.as_ref().unwrap()["new_line"],
            serde_json::json!(17)
        );
    }

    #[test]
    fn should_not_infer_outdated_when_either_head_is_missing_or_empty() {
        for (position_head, current_head) in [
            (None, Some("head-current")),
            (Some(""), Some("head-current")),
            (Some("head-old"), None),
            (Some("head-old"), Some("")),
        ] {
            let mut value = serde_json::json!({
                "position_type": "text",
                "new_path": "src/lib.rs",
                "new_line": 12
            });
            if let Some(position_head) = position_head {
                value["head_sha"] = serde_json::json!(position_head);
            }
            let thread = positional_discussion("missing-head", value)
                .into_review_thread(current_head)
                .unwrap();
            assert!(!thread.is_outdated);
        }
    }

    #[test]
    fn should_skip_discussion_without_position() {
        // A non-individual_note discussion without a diff position is dropped.
        let discussion = GlabDiscussion {
            id: "disc-2".to_string(),
            individual_note: false,
            notes: vec![GlabNote {
                id: 101,
                body: "general comment".to_string(),
                author: GlabNoteAuthor::default(),
                created_at: None,
                commit_id: None,
                position: None,
                resolved: false,
                system: false,
            }],
        };
        assert!(discussion.into_review_thread(None).is_none());
    }

    #[test]
    fn should_convert_individual_note_discussion_to_review_level_thread() {
        let discussion = GlabDiscussion {
            id: "disc-3".to_string(),
            individual_note: true,
            notes: vec![GlabNote {
                id: 200,
                body: "[NOTE] review comments".to_string(),
                author: GlabNoteAuthor {
                    username: "alice".to_string(),
                    name: "Alice".to_string(),
                },
                created_at: None,
                commit_id: None,
                position: None,
                resolved: false,
                system: false,
            }],
        };
        let thread = discussion.into_review_thread(None).unwrap();
        assert_eq!(thread.id, "disc-3");
        assert_eq!(thread.path, "");
        assert_eq!(thread.line, None);
        assert_eq!(thread.side, RemoteCommentSide::Right);
        assert!(!thread.is_resolved);
        assert_eq!(thread.comments[0].author.as_deref(), Some("alice"));
        assert_eq!(thread.comments[0].body, "[NOTE] review comments");
    }

    #[test]
    fn should_skip_individual_note_system_events() {
        let discussion = GlabDiscussion {
            id: "disc-4".to_string(),
            individual_note: true,
            notes: vec![GlabNote {
                id: 201,
                body: "requested review from @bob".to_string(),
                author: GlabNoteAuthor::default(),
                created_at: None,
                commit_id: None,
                position: None,
                resolved: false,
                system: true,
            }],
        };
        assert!(discussion.into_review_thread(None).is_none());
    }

    fn note(id: u64, body: &str, username: &str) -> GlabNote {
        GlabNote {
            id,
            body: body.to_string(),
            author: GlabNoteAuthor {
                username: username.to_string(),
                name: username.to_string(),
            },
            created_at: None,
            commit_id: None,
            position: None,
            resolved: false,
            system: false,
        }
    }

    #[test]
    fn should_group_positional_discussion_replies_under_root_note_id() {
        // Mandatory constraint (parity audit §3): GitLab discussions are
        // flat — every reply in a discussion belongs to the same thread,
        // so `in_reply_to` must point at the root note's own ID for every
        // note after the first, never hardcoded `None`.
        let mut root = note(300, "please fix this", "reviewer");
        root.position = Some(position(serde_json::json!({
            "position_type": "text",
            "new_path": "src/lib.rs",
            "new_line": 10
        })));
        let discussion = GlabDiscussion {
            id: "disc-5".to_string(),
            individual_note: false,
            notes: vec![
                root,
                note(301, "on it", "author"),
                note(302, "done, please re-check", "author"),
            ],
        };
        let thread = discussion.into_review_thread(None).unwrap();
        assert_eq!(thread.comments.len(), 3);
        assert_eq!(thread.comments[0].id, "300");
        assert_eq!(thread.comments[0].in_reply_to, None);
        assert_eq!(thread.comments[1].in_reply_to.as_deref(), Some("300"));
        assert_eq!(thread.comments[2].in_reply_to.as_deref(), Some("300"));
    }

    #[test]
    fn should_group_individual_note_discussion_replies_under_root_note_id() {
        let discussion = GlabDiscussion {
            id: "disc-6".to_string(),
            individual_note: true,
            notes: vec![
                note(400, "overall looks good", "reviewer"),
                note(401, "thanks!", "author"),
            ],
        };
        let thread = discussion.into_review_thread(None).unwrap();
        assert_eq!(thread.comments.len(), 2);
        assert_eq!(thread.comments[0].in_reply_to, None);
        assert_eq!(thread.comments[1].in_reply_to.as_deref(), Some("400"));
    }
}
