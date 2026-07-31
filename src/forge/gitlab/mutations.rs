//! Durable-thread write operations for GitLab: create a single merge
//! request discussion (with or without a diff position), reply to one, and
//! resolve/reopen it.
//!
//! Distinct from `glab.rs`'s batch `create_review` path (which posts a
//! general MR note for the review body via the plain Notes API and one
//! `/discussions` call per inline comment as part of submitting a whole
//! review): these three calls each act immediately, outside any batch
//! submission, matching `crate::forge::dryrun::plan_publication`'s
//! per-thread `CreateThread`/`Reply`/`Resolve`/`Reopen` operations.
//!
//! ## Evidence
//!
//! - `POST /projects/:id/merge_requests/:merge_request_iid/discussions` —
//!   "Create a merge request thread". With a `position[...]` payload this
//!   anchors to a diff line/range; *without* one it creates a top-level
//!   ("overview page") thread. Both forms return a discussion object whose
//!   root note (`notes[0]`) reports `"resolvable": true` for merge
//!   requests — unlike GitHub, a GitLab general/overview thread *is*
//!   natively resolvable, so (unlike the GitHub general/issue-comment case)
//!   no `"resolvable": false` downgrade is needed here:
//!   <https://docs.gitlab.com/api/discussions/#create-a-merge-request-thread>.
//! - `POST /projects/:id/merge_requests/:merge_request_iid/discussions/:discussion_id/notes`
//!   — "Add note to a merge request thread", used for replies:
//!   <https://docs.gitlab.com/api/discussions/#add-note-to-a-merge-request-thread>.
//! - `PUT /projects/:id/merge_requests/:merge_request_iid/discussions/:discussion_id?resolved=true|false`
//!   — "Resolve a merge request thread". Returns the updated discussion
//!   object; each note's own `resolved` flag reflects the new state:
//!   <https://docs.gitlab.com/api/discussions/#resolve-a-merge-request-thread>.
//! - Diff position shape (`base_sha`/`start_sha`/`head_sha`/`old_path`/
//!   `new_path`/`old_line`/`new_line`/`line_range`) mirrors the one
//!   `glab.rs`'s `create_review` already builds per inline comment:
//!   <https://docs.gitlab.com/api/discussions/#create-a-new-thread-in-the-merge-request-diff>.

use crate::error::{Result, TuicrError};
use crate::forge::submit::GhSide;
use crate::forge::traits::{
    CreateThreadResponse, NewThreadRequest, PullRequestDetails, ReplyResponse,
};
use crate::model::thread::AnchorSide;

use super::glab::{gl_api_hostname_args, gl_project_path, gl_range_endpoint};
use super::models::GlabDiscussion;

fn gl_side(side: AnchorSide) -> Result<GhSide> {
    match side {
        AnchorSide::Old => Ok(GhSide::Left),
        AnchorSide::New => Ok(GhSide::Right),
        AnchorSide::Both => Err(TuicrError::UnsupportedOperation(
            "GitLab diff notes anchor to exactly one diff side; \"both\" has no native \
             representation"
                .to_string(),
        )),
    }
}

/// Build the `glab api` args + JSON stdin body for
/// [`super::glab::GitLabGlabBackend::create_thread`]. Always targets the
/// `/discussions` endpoint (see this module's doc comment for why a
/// general/no-path comment does *not* need a separate, less-capable
/// endpoint the way GitHub's issue comments do). Returns the `(args,
/// body_json)` pair; the caller posts it via `run_with_stdin`.
pub(crate) fn build_create_thread_request(
    pr: &PullRequestDetails,
    request: &NewThreadRequest<'_>,
) -> Result<(Vec<String>, String)> {
    let project = gl_project_path(&pr.repository.owner, &pr.repository.name);
    let endpoint = format!(
        "projects/{project}/merge_requests/{}/discussions",
        pr.number
    );

    let mut payload = serde_json::json!({ "body": request.body });

    if let Some(path) = request.path {
        let start_sha = pr
            .diff_start_sha
            .as_deref()
            .unwrap_or(&pr.base_sha)
            .to_string();
        let mut position = serde_json::json!({
            "position_type": "text",
            "base_sha": pr.base_sha,
            "start_sha": start_sha,
            "head_sha": request.commit_id,
            "old_path": path,
            "new_path": path,
        });

        if let Some(line) = request.line {
            let side = gl_side(request.side.unwrap_or(AnchorSide::New))?;
            match side {
                GhSide::Right => position["new_line"] = serde_json::json!(line),
                GhSide::Left => position["old_line"] = serde_json::json!(line),
            }
            if let Some(start_line) = request.range_start {
                let start_side = side;
                let start_endpoint = gl_range_endpoint(path, start_side, start_line);
                let end_endpoint = gl_range_endpoint(path, side, line);
                position["line_range"] = serde_json::json!({
                    "start": start_endpoint,
                    "end": end_endpoint,
                });
            }
        }
        // A whole-file comment (`path` set, `line: None`) has no further
        // fields to add: GitLab's `position` object with only the SHAs and
        // paths anchors the discussion to the file as a whole.
        payload["position"] = position;
    }

    let payload_json = serde_json::to_string(&payload)?;
    let mut args = vec![
        "api".to_string(),
        endpoint,
        "--method".to_string(),
        "POST".to_string(),
        "--header".to_string(),
        "Content-Type: application/json".to_string(),
        "--input".to_string(),
        "-".to_string(),
    ];
    args.extend(gl_api_hostname_args(&pr.repository));
    Ok((args, payload_json))
}

/// Parse the discussion object returned by a `/discussions` create call
/// into `(discussion_id, root_note_id)`.
pub(crate) fn parse_create_discussion_response(output: &str) -> Result<(String, u64)> {
    let discussion: GlabDiscussion = serde_json::from_str(output)?;
    let root_note_id = discussion
        .notes
        .first()
        .map(|note| note.id)
        .ok_or_else(|| {
            TuicrError::Forge("GitLab discussion response has no root note".to_string())
        })?;
    Ok((discussion.id, root_note_id))
}

/// Assemble the final [`CreateThreadResponse`] from a parsed discussion.
pub(crate) fn build_create_thread_response(
    discussion_id: String,
    root_note_id: u64,
) -> CreateThreadResponse {
    CreateThreadResponse {
        mapping: serde_json::json!({
            "id": discussion_id,
            "is_resolved": false,
        }),
        root_comment_id: root_note_id.to_string(),
    }
}

/// Build the args + body for
/// [`super::glab::GitLabGlabBackend::reply_to_thread`]. Reads the
/// discussion ID from `provider_mapping["id"]` (unlike GitHub, GitLab's
/// reply call addresses the *discussion*, not the root comment — so
/// `root_comment_id` is not needed here).
pub(crate) fn build_reply_request(
    pr: &PullRequestDetails,
    discussion_id: &str,
    body: &str,
) -> Result<(Vec<String>, String)> {
    let project = gl_project_path(&pr.repository.owner, &pr.repository.name);
    let endpoint = format!(
        "projects/{project}/merge_requests/{}/discussions/{discussion_id}/notes",
        pr.number
    );
    let payload_json = serde_json::to_string(&serde_json::json!({ "body": body }))?;
    let mut args = vec![
        "api".to_string(),
        endpoint,
        "--method".to_string(),
        "POST".to_string(),
        "--header".to_string(),
        "Content-Type: application/json".to_string(),
        "--input".to_string(),
        "-".to_string(),
    ];
    args.extend(gl_api_hostname_args(&pr.repository));
    Ok((args, payload_json))
}

pub(crate) fn parse_reply_response(output: &str) -> Result<ReplyResponse> {
    let value: serde_json::Value = serde_json::from_str(output)?;
    let id = value
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| TuicrError::Forge("GitLab note response missing `id`".to_string()))?;
    Ok(ReplyResponse {
        comment_id: id.to_string(),
    })
}

/// Build the args for
/// [`super::glab::GitLabGlabBackend::set_thread_resolution`]. Returns
/// `Err` up front (never sends a request) when `provider_mapping` is
/// stamped `"resolvable": false` — reserved for any future GitLab note
/// kind that turns out not to support resolution; today every discussion
/// this module creates is resolvable (see the module doc comment).
pub(crate) fn build_resolution_request(
    pr: &PullRequestDetails,
    provider_mapping: &serde_json::Value,
    resolved: bool,
) -> Result<Vec<String>> {
    if provider_mapping.get("resolvable") == Some(&serde_json::Value::Bool(false)) {
        return Err(TuicrError::UnsupportedOperation(
            "this GitLab thread was marked non-resolvable and cannot be resolved or reopened"
                .to_string(),
        ));
    }
    let discussion_id = provider_mapping
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TuicrError::Forge("provider mapping is missing the GitLab discussion `id`".to_string())
        })?;
    let project = gl_project_path(&pr.repository.owner, &pr.repository.name);
    let endpoint = format!(
        "projects/{project}/merge_requests/{}/discussions/{discussion_id}?resolved={resolved}",
        pr.number
    );
    let mut args = vec![
        "api".to_string(),
        endpoint,
        "--method".to_string(),
        "PUT".to_string(),
    ];
    args.extend(gl_api_hostname_args(&pr.repository));
    Ok(args)
}

/// Parse the discussion object returned by a resolve/reopen PUT call.
/// Returns the root note's `resolved` flag.
pub(crate) fn parse_resolution_response(output: &str) -> Result<bool> {
    let discussion: GlabDiscussion = serde_json::from_str(output)?;
    let resolved = discussion
        .notes
        .first()
        .map(|note| note.resolved)
        .ok_or_else(|| {
            TuicrError::Forge("GitLab discussion response has no root note".to_string())
        })?;
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    fn pr(host: &str) -> PullRequestDetails {
        PullRequestDetails {
            repository: ForgeRepository::gitlab(host, "agavra", "tuicr"),
            number: 42,
            title: "t".to_string(),
            url: format!("https://{host}/agavra/tuicr/-/merge_requests/42"),
            state: "OPEN".to_string(),
            is_draft: false,
            author: None,
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "head_sha_value".to_string(),
            base_sha: "base_sha_value".to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: Some("start_sha_value".to_string()),
        }
    }

    fn new_thread(
        path: Option<&'static str>,
        line: Option<u32>,
        side: Option<AnchorSide>,
    ) -> NewThreadRequest<'static> {
        NewThreadRequest {
            commit_id: "head_sha_value",
            body: "hello",
            path,
            line,
            side,
            range_start: None,
        }
    }

    #[test]
    fn should_build_line_comment_position_with_side() {
        let (args, body) = build_create_thread_request(
            &pr("gitlab.com"),
            &new_thread(Some("src/a.rs"), Some(10), Some(AnchorSide::New)),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["position"]["new_line"], 10);
        assert_eq!(value["position"]["base_sha"], "base_sha_value");
        assert_eq!(value["position"]["start_sha"], "start_sha_value");
        assert_eq!(value["position"]["head_sha"], "head_sha_value");
        assert!(
            args.contains(&"projects/agavra%2Ftuicr/merge_requests/42/discussions".to_string())
        );
        // Default host: no --hostname flag.
        assert!(!args.contains(&"--hostname".to_string()));
    }

    #[test]
    fn should_build_range_comment_with_line_range() {
        let mut request = new_thread(Some("src/a.rs"), Some(12), Some(AnchorSide::New));
        request.range_start = Some(10);
        let (_, body) = build_create_thread_request(&pr("gitlab.com"), &request).unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["position"]["line_range"]["start"]["new_line"], 10);
        assert_eq!(value["position"]["line_range"]["end"]["new_line"], 12);
    }

    #[test]
    fn should_build_file_comment_with_no_line() {
        let (_, body) = build_create_thread_request(
            &pr("gitlab.com"),
            &new_thread(Some("src/a.rs"), None, None),
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(value["position"]["new_line"].is_null());
        assert!(value["position"]["old_line"].is_null());
        assert_eq!(value["position"]["new_path"], "src/a.rs");
    }

    #[test]
    fn should_build_general_comment_with_no_position() {
        let (_, body) =
            build_create_thread_request(&pr("gitlab.com"), &new_thread(None, None, None)).unwrap();
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(value.get("position").is_none());
        assert_eq!(value["body"], "hello");
    }

    #[test]
    fn should_reject_both_side_anchor_with_no_native_representation() {
        let err = build_create_thread_request(
            &pr("gitlab.com"),
            &new_thread(Some("src/a.rs"), Some(10), Some(AnchorSide::Both)),
        )
        .unwrap_err();
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_route_self_hosted_host_via_hostname_flag() {
        let (args, _) =
            build_create_thread_request(&pr("gitlab.example.com"), &new_thread(None, None, None))
                .unwrap();
        assert!(args.contains(&"--hostname".to_string()));
        assert!(args.contains(&"gitlab.example.com".to_string()));
    }

    #[test]
    fn should_parse_create_discussion_response() {
        let output = serde_json::json!({
            "id": "abc123",
            "individual_note": false,
            "notes": [{"id": 555, "body": "hello", "resolved": false}],
        })
        .to_string();
        let (discussion_id, root_note_id) = parse_create_discussion_response(&output).unwrap();
        assert_eq!(discussion_id, "abc123");
        assert_eq!(root_note_id, 555);
    }

    #[test]
    fn should_build_mapping_with_is_resolved_false_after_create() {
        let response = build_create_thread_response("abc123".to_string(), 555);
        assert_eq!(response.mapping["id"], "abc123");
        assert_eq!(response.mapping["is_resolved"], false);
        assert_eq!(response.root_comment_id, "555");
    }

    #[test]
    fn should_build_reply_request_against_discussion_id() {
        let (args, body) = build_reply_request(&pr("gitlab.com"), "abc123", "reply body").unwrap();
        assert!(args.contains(
            &"projects/agavra%2Ftuicr/merge_requests/42/discussions/abc123/notes".to_string()
        ));
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["body"], "reply body");
    }

    #[test]
    fn should_parse_reply_response() {
        let output = serde_json::json!({"id": 999, "body": "reply body"}).to_string();
        let reply = parse_reply_response(&output).unwrap();
        assert_eq!(reply.comment_id, "999");
    }

    #[test]
    fn should_build_resolve_and_reopen_requests() {
        let mapping = serde_json::json!({"id": "abc123", "is_resolved": false});
        let resolve_args = build_resolution_request(&pr("gitlab.com"), &mapping, true).unwrap();
        assert!(
            resolve_args
                .iter()
                .any(|a| a.contains("discussions/abc123?resolved=true"))
        );
        let reopen_args = build_resolution_request(&pr("gitlab.com"), &mapping, false).unwrap();
        assert!(
            reopen_args
                .iter()
                .any(|a| a.contains("discussions/abc123?resolved=false"))
        );
    }

    #[test]
    fn should_refuse_resolution_up_front_when_mapping_marks_non_resolvable() {
        let mapping =
            serde_json::json!({"id": "abc123", "is_resolved": false, "resolvable": false});
        let err = build_resolution_request(&pr("gitlab.com"), &mapping, true).unwrap_err();
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_parse_successful_resolution_response() {
        let output = serde_json::json!({
            "id": "abc123",
            "notes": [{"id": 555, "body": "hello", "resolved": true}],
        })
        .to_string();
        assert!(parse_resolution_response(&output).unwrap());
    }
}
