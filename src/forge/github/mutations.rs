//! Durable-thread write operations for GitHub: create a single review
//! comment/thread, reply to one, resolve/unresolve a thread, and add an
//! incremental comment to an already-created pending (draft) review.
//!
//! Distinct from `submit.rs`'s batch `create_review` path (pending/draft
//! review creation): these three calls each act immediately, outside any
//! pending review, matching `crate::forge::dryrun::plan_publication`'s
//! per-thread `CreateThread`/`Reply`/`Resolve`/`Reopen` operations.
//!
//! ## Evidence
//!
//! - `POST /repos/{owner}/{repo}/pulls/{pull_number}/comments` — "Create a
//!   review comment for a pull request":
//!   <https://docs.github.com/en/rest/pulls/comments?apiVersion=2022-11-28#create-a-review-comment-for-a-pull-request>.
//!   Accepts `body`/`commit_id`/`path` plus either `line`/`side` (and
//!   optional `start_line`/`start_side` for a range) for a line comment, or
//!   `subject_type: "file"` (lowercase — the API rejects `"FILE"`/`"LINE"`)
//!   for a whole-file comment with no line. `in_reply_to` (a comment's
//!   numeric `id`, not its GraphQL node ID) replies to an existing comment
//!   and must not be combined with `path`/`line`/`side`/`subject_type`.
//! - `POST /repos/{owner}/{repo}/issues/{issue_number}/comments` — "Create
//!   an issue comment", used for pull request review-level general
//!   comments outside a review (has no anchor and is not part of any
//!   `reviewThreads` connection, so it cannot later be resolved):
//!   <https://docs.github.com/en/rest/issues/comments?apiVersion=2022-11-28#create-an-issue-comment>.
//! - `resolveReviewThread`/`unresolveReviewThread` GraphQL mutations, plus
//!   the pull request's paginated `reviewThreads` connection (used to
//!   discover a just-created comment's owning thread ID, which the REST
//!   create-comment response does not include):
//!   <https://docs.github.com/en/graphql/reference/mutations#resolvereviewthread>,
//!   <https://docs.github.com/en/graphql/reference/mutations#unresolvereviewthread>,
//!   <https://docs.github.com/en/graphql/reference/objects#pullrequestreviewcomment>.
//! - `POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews/{review_id}/comments`
//!   — the plain (non-`in_reply_to`) form of the review-comment create
//!   endpoint, scoped under a specific `review_id`, is GitHub's documented
//!   way to add another comment to a pending review that has already been
//!   created (via `create_review` with no `event`, i.e.
//!   `SubmitEvent::Draft`) but not yet submitted — see the pending-review
//!   lifecycle described at
//!   <https://docs.github.com/en/rest/pulls/reviews?apiVersion=2022-11-28#create-a-review-for-a-pull-request>
//!   and the comment-create shape at
//!   <https://docs.github.com/en/rest/pulls/comments?apiVersion=2022-11-28#create-a-review-comment-for-a-pull-request>.

use crate::error::{Result, TuicrError};
use crate::forge::traits::{
    CreateThreadResponse, NewThreadRequest, PullRequestDetails, ReplyResponse,
};
use crate::model::thread::AnchorSide;

const DEFAULT_GITHUB_HOST: &str = "github.com";

fn gh_side(side: AnchorSide) -> Result<&'static str> {
    match side {
        AnchorSide::Old => Ok("LEFT"),
        AnchorSide::New => Ok("RIGHT"),
        AnchorSide::Both => Err(TuicrError::UnsupportedOperation(
            "GitHub review comments anchor to exactly one diff side; \"both\" has no native \
             representation"
                .to_string(),
        )),
    }
}

/// Build the anchored-comment JSON payload shared by
/// [`build_create_thread_request`]'s inline branch and
/// [`build_add_pending_review_comment_request`]: `body`/`commit_id`/`path`
/// plus either `line`/`side` (and optional `start_line`/`start_side` for a
/// range) or `subject_type: "file"` for a whole-file comment.
fn build_anchor_payload(request: &NewThreadRequest<'_>, path: &str) -> Result<serde_json::Value> {
    let mut payload = serde_json::json!({
        "body": request.body,
        "commit_id": request.commit_id,
        "path": path,
    });
    match request.line {
        None => {
            payload["subject_type"] = serde_json::Value::String("file".to_string());
        }
        Some(line) => {
            let side = gh_side(request.side.unwrap_or(AnchorSide::New))?;
            payload["line"] = serde_json::json!(line);
            payload["side"] = serde_json::Value::String(side.to_string());
            if let Some(start_line) = request.range_start {
                payload["start_line"] = serde_json::json!(start_line);
                payload["start_side"] = serde_json::Value::String(side.to_string());
            }
        }
    }
    Ok(payload)
}

/// Build the `gh api` args + JSON body for [`super::gh::GitHubGhBackend::create_thread`].
/// Returns `(args, body_json, is_general_comment)` — `is_general_comment`
/// is true for a review-level (no-path) comment, which uses the issue
/// -comments endpoint and produces a mapping that can never be resolved
/// (see this module's doc comment).
pub(crate) fn build_create_thread_request(
    pr: &PullRequestDetails,
    request: &NewThreadRequest<'_>,
) -> Result<(Vec<String>, String, bool)> {
    let is_general = request.path.is_none();
    let endpoint = if is_general {
        format!(
            "repos/{}/{}/issues/{}/comments",
            pr.repository.owner, pr.repository.name, pr.number
        )
    } else {
        format!(
            "repos/{}/{}/pulls/{}/comments",
            pr.repository.owner, pr.repository.name, pr.number
        )
    };

    let payload = if let Some(path) = request.path {
        build_anchor_payload(request, path)?
    } else {
        serde_json::json!({ "body": request.body })
    };

    let payload_json = serde_json::to_string(&payload)?;
    let mut args = vec![
        "api".to_string(),
        endpoint,
        "--method".to_string(),
        "POST".to_string(),
        "--input".to_string(),
        "-".to_string(),
    ];
    if pr.repository.host != DEFAULT_GITHUB_HOST {
        args.push("--hostname".to_string());
        args.push(pr.repository.host.clone());
    }
    Ok((args, payload_json, is_general))
}

/// Build the `gh api` args + JSON body for
/// [`super::gh::GitHubGhBackend::add_comment_to_pending_review`]:
/// `POST /repos/{owner}/{repo}/pulls/{pull_number}/reviews/{review_id}/comments`
/// — "Add a review comment to a pending review" (an already-created but
/// not-yet-submitted review, per
/// <https://docs.github.com/en/rest/pulls/reviews?apiVersion=2022-11-28#update-a-review-comment-for-a-pull-request>
/// and the pending-review lifecycle documented at
/// <https://docs.github.com/en/rest/pulls/reviews?apiVersion=2022-11-28#create-a-review-for-a-pull-request>).
/// Distinct from [`build_create_thread_request`]: it targets one specific
/// pending review by numeric ID instead of posting a standalone comment
/// outside any review, and (matching the real endpoint) always requires an
/// anchor — a review-level general comment cannot be added to a pending
/// review this way, so this returns `Err` up front when `request.path` is
/// `None` rather than silently falling back to the issue-comments endpoint.
pub(crate) fn build_add_pending_review_comment_request(
    pr: &PullRequestDetails,
    pending_review_id: u64,
    request: &NewThreadRequest<'_>,
) -> Result<(Vec<String>, String)> {
    let path = request.path.ok_or_else(|| {
        TuicrError::UnsupportedOperation(
            "GitHub pending-review comments require an anchor (path); a review-level general \
             comment cannot be added to a pending review this way"
                .to_string(),
        )
    })?;
    let payload = build_anchor_payload(request, path)?;
    let payload_json = serde_json::to_string(&payload)?;
    let endpoint = format!(
        "repos/{}/{}/pulls/{}/reviews/{}/comments",
        pr.repository.owner, pr.repository.name, pr.number, pending_review_id
    );
    let mut args = vec![
        "api".to_string(),
        endpoint,
        "--method".to_string(),
        "POST".to_string(),
        "--input".to_string(),
        "-".to_string(),
    ];
    if pr.repository.host != DEFAULT_GITHUB_HOST {
        args.push("--hostname".to_string());
        args.push(pr.repository.host.clone());
    }
    Ok((args, payload_json))
}

/// Parse the REST response shared by both the review-comment and
/// issue-comment create endpoints: both return an object with numeric `id`
/// and string `node_id`.
pub(crate) fn parse_create_comment_response(output: &str) -> Result<(u64, String)> {
    let value: serde_json::Value = serde_json::from_str(output)?;
    let id = value
        .get("id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| TuicrError::Forge("GitHub comment response missing `id`".to_string()))?;
    let node_id = value
        .get("node_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| TuicrError::Forge("GitHub comment response missing `node_id`".to_string()))?
        .to_string();
    Ok((id, node_id))
}

/// Build the args for the follow-up GraphQL lookup that discovers a
/// just-created review comment's owning thread ID (the REST create
/// response has no thread reference at all).
pub(crate) fn build_thread_lookup_args(
    pr: &PullRequestDetails,
    cursor: Option<&str>,
) -> Vec<String> {
    let query = "query($owner: String!, $name: String!, $number: Int!, $after: String) { \
                 repository(owner: $owner, name: $name) { pullRequest(number: $number) { \
                 reviewThreads(first: 100, after: $after) { nodes { id isResolved \
                 comments(first: 1) { nodes { id } } } pageInfo { hasNextPage endCursor } } } } }";
    let mut args = vec![
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={query}"),
        "-F".to_string(),
        format!("owner={}", pr.repository.owner),
        "-F".to_string(),
        format!("name={}", pr.repository.name),
        "-F".to_string(),
        format!("number={}", pr.number),
    ];
    if let Some(cursor) = cursor {
        args.push("-F".to_string());
        args.push(format!("after={cursor}"));
    }
    if pr.repository.host != DEFAULT_GITHUB_HOST {
        args.push("--hostname".to_string());
        args.push(pr.repository.host.clone());
    }
    args
}

pub(crate) struct ThreadLookupPage {
    pub thread: Option<(String, bool)>,
    pub next_cursor: Option<String>,
}

/// Parse one page of the thread-lookup GraphQL response.
pub(crate) fn parse_thread_lookup_response(
    output: &str,
    comment_node_id: &str,
) -> Result<ThreadLookupPage> {
    let value: serde_json::Value = serde_json::from_str(output)?;
    if let Some(errors) = value.get("errors").and_then(|errors| errors.as_array())
        && !errors.is_empty()
    {
        return Err(TuicrError::Forge(format!(
            "GitHub thread lookup returned GraphQL errors: {}",
            serde_json::to_string(errors)?
        )));
    }
    let connection = value
        .pointer("/data/repository/pullRequest/reviewThreads")
        .ok_or_else(|| {
            TuicrError::Forge("GitHub thread lookup missing reviewThreads".to_string())
        })?;
    let thread = connection
        .get("nodes")
        .and_then(|nodes| nodes.as_array())
        .and_then(|nodes| {
            nodes.iter().find(|thread| {
                thread
                    .pointer("/comments/nodes/0/id")
                    .and_then(|id| id.as_str())
                    == Some(comment_node_id)
            })
        })
        .map(|thread| {
            let id = thread.get("id").and_then(|id| id.as_str()).ok_or_else(|| {
                TuicrError::Forge("GitHub thread lookup missing `id`".to_string())
            })?;
            let is_resolved = thread
                .get("isResolved")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            Ok::<_, TuicrError>((id.to_string(), is_resolved))
        })
        .transpose()?;
    let next_cursor = if connection
        .pointer("/pageInfo/hasNextPage")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        Some(
            connection
                .pointer("/pageInfo/endCursor")
                .and_then(|value| value.as_str())
                .ok_or_else(|| {
                    TuicrError::Forge("GitHub thread lookup page has no `endCursor`".to_string())
                })?
                .to_string(),
        )
    } else {
        None
    };
    Ok(ThreadLookupPage {
        thread,
        next_cursor,
    })
}

/// Assemble the final [`CreateThreadResponse`] once the comment has been
/// created and (for anchored comments) its owning thread resolved.
pub(crate) fn build_create_thread_response(
    comment_id: u64,
    comment_node_id: &str,
    thread: Option<(String, bool)>,
) -> CreateThreadResponse {
    let mapping = match &thread {
        Some((thread_id, is_resolved)) => serde_json::json!({
            "id": thread_id,
            "is_resolved": is_resolved,
            "root_comment_id": comment_id.to_string(),
        }),
        None => serde_json::json!({
            "id": comment_node_id,
            "is_resolved": false,
            "root_comment_id": comment_id.to_string(),
            // No GitHub reviewThread exists for a general/issue comment;
            // `set_thread_resolution` must refuse rather than send a
            // doomed `resolveReviewThread` mutation against a non-thread ID.
            "resolvable": false,
        }),
    };
    CreateThreadResponse {
        mapping,
        root_comment_id: comment_id.to_string(),
    }
}

/// Build the args + body for [`super::gh::GitHubGhBackend::reply_to_thread`].
pub(crate) fn build_reply_request(
    pr: &PullRequestDetails,
    root_comment_id: &str,
    body: &str,
) -> Result<(Vec<String>, String)> {
    let endpoint = format!(
        "repos/{}/{}/pulls/{}/comments",
        pr.repository.owner, pr.repository.name, pr.number
    );
    let payload = serde_json::json!({
        "body": body,
        "in_reply_to": root_comment_id.parse::<u64>().map_err(|_| {
            TuicrError::Forge(format!(
                "provider mapping's `root_comment_id` ({root_comment_id:?}) is not a valid GitHub \
                 comment ID"
            ))
        })?,
    });
    let payload_json = serde_json::to_string(&payload)?;
    let mut args = vec![
        "api".to_string(),
        endpoint,
        "--method".to_string(),
        "POST".to_string(),
        "--input".to_string(),
        "-".to_string(),
    ];
    if pr.repository.host != DEFAULT_GITHUB_HOST {
        args.push("--hostname".to_string());
        args.push(pr.repository.host.clone());
    }
    Ok((args, payload_json))
}

pub(crate) fn parse_reply_response(output: &str) -> Result<ReplyResponse> {
    let (id, _node_id) = parse_create_comment_response(output)?;
    Ok(ReplyResponse {
        comment_id: id.to_string(),
    })
}

/// Build the args for [`super::gh::GitHubGhBackend::set_thread_resolution`].
/// Returns `Err` up front (never sends a request) when `provider_mapping`
/// is stamped `"resolvable": false` (see
/// [`build_create_thread_response`]) — a general/issue comment has no
/// `PullRequestReviewThread` to resolve.
pub(crate) fn build_resolution_request(
    pr: &PullRequestDetails,
    provider_mapping: &serde_json::Value,
    resolved: bool,
) -> Result<Vec<String>> {
    if provider_mapping.get("resolvable") == Some(&serde_json::Value::Bool(false)) {
        return Err(TuicrError::UnsupportedOperation(
            "this thread has no GitHub reviewThread (it was created as a general/issue comment) \
             and cannot be resolved or reopened"
                .to_string(),
        ));
    }
    let thread_id = provider_mapping
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            TuicrError::Forge("provider mapping is missing the GitHub thread `id`".to_string())
        })?;
    let mutation_name = if resolved {
        "resolveReviewThread"
    } else {
        "unresolveReviewThread"
    };
    let query = format!(
        "mutation($id: ID!) {{ {mutation_name}(input: {{ threadId: $id }}) {{ thread {{ id \
         isResolved }} }} }}"
    );
    let mut args = vec![
        "api".to_string(),
        "graphql".to_string(),
        "-f".to_string(),
        format!("query={query}"),
        "-f".to_string(),
        format!("id={thread_id}"),
    ];
    if pr.repository.host != DEFAULT_GITHUB_HOST {
        args.push("--hostname".to_string());
        args.push(pr.repository.host.clone());
    }
    Ok(args)
}

/// Inspect a `resolveReviewThread`/`unresolveReviewThread` GraphQL response
/// for top-level errors (GraphQL mutations return HTTP 200 even on
/// failure). Returns `Ok(is_resolved)` on success.
pub(crate) fn parse_resolution_response(output: &str, mutation_name: &str) -> Result<bool> {
    let value: serde_json::Value = serde_json::from_str(output)?;
    if let Some(errors) = value.get("errors").and_then(|v| v.as_array())
        && !errors.is_empty()
    {
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|e| e.get("message").and_then(|m| m.as_str()))
            .map(str::to_string)
            .collect();
        return Err(TuicrError::Forge(format!(
            "GitHub {mutation_name} failed: {}",
            messages.join(", ")
        )));
    }
    let is_resolved = value
        .pointer(&format!("/data/{mutation_name}/thread/isResolved"))
        .and_then(|v| v.as_bool())
        .ok_or_else(|| {
            TuicrError::Forge(format!(
                "GitHub {mutation_name} response missing `thread.isResolved`"
            ))
        })?;
    Ok(is_resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::traits::ForgeRepository;

    fn pr(path_host: &str) -> PullRequestDetails {
        PullRequestDetails {
            repository: ForgeRepository::github(path_host, "agavra", "tuicr"),
            number: 125,
            title: "t".to_string(),
            url: "https://github.com/agavra/tuicr/pull/125".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: None,
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "headsha".to_string(),
            base_sha: "basesha".to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: None,
        }
    }

    #[test]
    fn should_build_line_comment_payload_with_side() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "Can this be simplified?",
            path: Some("src/lib.rs"),
            line: Some(42),
            side: Some(AnchorSide::New),
            range_start: None,
        };
        let (args, body, is_general) = build_create_thread_request(&pr("github.com"), &request)
            .expect("line comment request should build");
        assert!(!is_general);
        assert!(args.iter().any(|a| a.contains("/pulls/125/comments")));
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(payload["line"], serde_json::json!(42));
        assert_eq!(payload["side"], serde_json::json!("RIGHT"));
        assert_eq!(payload["commit_id"], serde_json::json!("headsha"));
        assert!(payload.get("subject_type").is_none());
    }

    #[test]
    fn should_build_file_comment_payload_with_subject_type_file() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "Whole file needs docs",
            path: Some("src/lib.rs"),
            line: None,
            side: None,
            range_start: None,
        };
        let (_args, body, is_general) = build_create_thread_request(&pr("github.com"), &request)
            .expect("file comment request should build");
        assert!(!is_general);
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(payload["subject_type"], serde_json::json!("file"));
        assert!(payload.get("line").is_none());
    }

    #[test]
    fn should_build_range_comment_payload_with_start_line_and_side() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "range",
            path: Some("src/lib.rs"),
            line: Some(50),
            side: Some(AnchorSide::New),
            range_start: Some(45),
        };
        let (_args, body, _) = build_create_thread_request(&pr("github.com"), &request).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(payload["start_line"], serde_json::json!(45));
        assert_eq!(payload["start_side"], serde_json::json!("RIGHT"));
        assert_eq!(payload["line"], serde_json::json!(50));
    }

    #[test]
    fn should_reject_both_side_anchor_with_no_native_representation() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "both",
            path: Some("src/lib.rs"),
            line: Some(1),
            side: Some(AnchorSide::Both),
            range_start: None,
        };
        let err = build_create_thread_request(&pr("github.com"), &request).unwrap_err();
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_use_issue_comments_endpoint_for_review_level_general_comment() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "General note",
            path: None,
            line: None,
            side: None,
            range_start: None,
        };
        let (args, body, is_general) = build_create_thread_request(&pr("github.com"), &request)
            .expect("general comment request should build");
        assert!(is_general);
        assert!(args.iter().any(|a| a.contains("/issues/125/comments")));
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(payload.get("commit_id").is_none());
        assert!(payload.get("path").is_none());
    }

    #[test]
    fn should_route_enterprise_host_via_hostname_flag() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "note",
            path: Some("a.rs"),
            line: Some(1),
            side: Some(AnchorSide::New),
            range_start: None,
        };
        let (args, _, _) =
            build_create_thread_request(&pr("github.example.com"), &request).unwrap();
        assert!(args.iter().any(|a| a == "--hostname"));
        assert!(args.iter().any(|a| a == "github.example.com"));
    }

    const CREATE_COMMENT_RESPONSE_JSON: &str = r#"{
        "id": 999,
        "node_id": "PRRC_kwABC",
        "body": "Can this be simplified?"
    }"#;

    #[test]
    fn should_parse_create_comment_response() {
        let (id, node_id) = parse_create_comment_response(CREATE_COMMENT_RESPONSE_JSON).unwrap();
        assert_eq!(id, 999);
        assert_eq!(node_id, "PRRC_kwABC");
    }

    const THREAD_LOOKUP_RESPONSE_JSON: &str = r#"{
        "data": {"repository": {"pullRequest": {"reviewThreads": {
            "nodes": [{
                "id": "PRRT_kwXYZ",
                "isResolved": false,
                "comments": {"nodes": [{"id": "PRRC_kwABC"}]}
            }],
            "pageInfo": {"hasNextPage": false, "endCursor": null}
        }}}}
    }"#;

    #[test]
    fn should_parse_thread_lookup_response() {
        let result = parse_thread_lookup_response(THREAD_LOOKUP_RESPONSE_JSON, "PRRC_kwABC")
            .unwrap()
            .thread
            .expect("thread lookup should find a thread");
        assert_eq!(result.0, "PRRT_kwXYZ");
        assert!(!result.1);
    }

    #[test]
    fn should_return_none_when_lookup_response_has_no_thread() {
        let json = r#"{"data":{"repository":{"pullRequest":{"reviewThreads":{
            "nodes":[], "pageInfo":{"hasNextPage":false,"endCursor":null}
        }}}}}"#;
        let page = parse_thread_lookup_response(json, "PRRC_missing").unwrap();
        assert_eq!(page.thread, None);
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn should_build_line_anchored_mapping_with_resolvable_thread() {
        let response = build_create_thread_response(
            999,
            "PRRC_kwABC",
            Some(("PRRT_kwXYZ".to_string(), false)),
        );
        assert_eq!(response.mapping["id"], serde_json::json!("PRRT_kwXYZ"));
        assert_eq!(response.mapping["is_resolved"], serde_json::json!(false));
        assert_eq!(response.root_comment_id, "999");
    }

    #[test]
    fn should_mark_general_comment_mapping_unresolvable() {
        let response = build_create_thread_response(1, "IC_kwABC", None);
        assert_eq!(response.mapping["resolvable"], serde_json::json!(false));
        let err = build_resolution_request(&pr("github.com"), &response.mapping, true).unwrap_err();
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_build_reply_request_with_numeric_in_reply_to() {
        let (args, body) = build_reply_request(&pr("github.com"), "999", "+1").unwrap();
        assert!(args.iter().any(|a| a.contains("/pulls/125/comments")));
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(payload["in_reply_to"], serde_json::json!(999));
        assert_eq!(payload["body"], serde_json::json!("+1"));
    }

    #[test]
    fn should_reject_non_numeric_root_comment_id_for_reply() {
        let err = build_reply_request(&pr("github.com"), "not-a-number", "+1").unwrap_err();
        assert!(matches!(err, TuicrError::Forge(_)));
    }

    #[test]
    fn should_build_resolve_and_unresolve_mutations() {
        let mapping = serde_json::json!({"id": "PRRT_kwXYZ", "is_resolved": false});
        let resolve_args = build_resolution_request(&pr("github.com"), &mapping, true).unwrap();
        assert!(
            resolve_args
                .iter()
                .any(|a| a.starts_with("query=") && a.contains("resolveReviewThread"))
        );
        let reopen_args = build_resolution_request(&pr("github.com"), &mapping, false).unwrap();
        assert!(
            reopen_args
                .iter()
                .any(|a| a.starts_with("query=") && a.contains("unresolveReviewThread"))
        );
    }

    const RESOLVE_RESPONSE_JSON: &str = r#"{
        "data": { "resolveReviewThread": { "thread": { "id": "PRRT_kwXYZ", "isResolved": true } } }
    }"#;

    #[test]
    fn should_parse_successful_resolution_response() {
        let is_resolved =
            parse_resolution_response(RESOLVE_RESPONSE_JSON, "resolveReviewThread").unwrap();
        assert!(is_resolved);
    }

    const RESOLVE_ERROR_RESPONSE_JSON: &str = r#"{
        "errors": [{"message": "Could not resolve to a node with the global id of 'bogus'."}]
    }"#;

    #[test]
    fn should_surface_graphql_errors_on_resolution_failure() {
        let err = parse_resolution_response(RESOLVE_ERROR_RESPONSE_JSON, "resolveReviewThread")
            .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("Could not resolve to a node"));
    }

    #[test]
    fn should_build_pending_review_comment_request_targeting_review_id() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "incremental pending-review comment",
            path: Some("src/lib.rs"),
            line: Some(10),
            side: Some(AnchorSide::New),
            range_start: None,
        };
        let (args, body) =
            build_add_pending_review_comment_request(&pr("github.com"), 555, &request)
                .expect("anchored pending-review comment request should build");
        assert!(
            args.iter()
                .any(|a| a.contains("/pulls/125/reviews/555/comments")),
            "expected reviews/555/comments endpoint, got {args:?}"
        );
        let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(payload["line"], serde_json::json!(10));
        assert_eq!(payload["side"], serde_json::json!("RIGHT"));
    }

    #[test]
    fn should_reject_general_comment_for_pending_review_endpoint() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "general comment",
            path: None,
            line: None,
            side: None,
            range_start: None,
        };
        let err =
            build_add_pending_review_comment_request(&pr("github.com"), 555, &request).unwrap_err();
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_include_hostname_for_pending_review_comment_on_enterprise_host() {
        let request = NewThreadRequest {
            commit_id: "headsha",
            body: "enterprise",
            path: Some("src/lib.rs"),
            line: Some(1),
            side: Some(AnchorSide::New),
            range_start: None,
        };
        let (args, _body) =
            build_add_pending_review_comment_request(&pr("github.example.com"), 1, &request)
                .expect("request should build for enterprise host");
        assert!(args.iter().any(|a| a == "--hostname"));
        assert!(args.iter().any(|a| a == "github.example.com"));
    }
}
