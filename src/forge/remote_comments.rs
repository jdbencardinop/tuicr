//! Remote review comment/thread models.
//!
//! These types carry existing provider review discussions into the App for
//! read-only display, filtering, export, and durable import. They remain
//! source-of-truth-on-remote DTOs: local reply/status overlays never mutate
//! them, while [`crate::model::thread_store::thread_from_remote`] copies their
//! normalized and provider-native anchor data into the canonical store.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

/// Which side of the diff a remote comment anchors to.
///
/// Mirrors GitHub's submission model: `RIGHT` is the head side (added/context
/// lines), `LEFT` is the base side (deleted lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteCommentSide {
    Right,
    Left,
}

impl RemoteCommentSide {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "LEFT" => RemoteCommentSide::Left,
            _ => RemoteCommentSide::Right,
        }
    }
}

/// A single remote review comment, fetched from a forge.
///
/// Anchor fields (`path`, `line`, `side`) live on the parent
/// `RemoteReviewThread`, not on each comment, mirroring GitHub's GraphQL
/// schema where `PullRequestReviewComment` does not carry these directly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteReviewComment {
    /// Forge-assigned comment node ID (opaque string).
    pub id: String,
    /// Login/handle of the comment author, when available.
    pub author: Option<String>,
    /// Markdown body as written on the forge.
    pub body: String,
    pub created_at: Option<DateTime<Utc>>,
    /// For reply comments, the ID of the parent comment.
    pub in_reply_to: Option<String>,
    /// Permalink to the comment on the forge.
    pub url: String,
    /// This comment's REST-API-compatible identifier, when it differs from
    /// [`Self::id`]. GitHub's review-thread import goes through GraphQL,
    /// whose node IDs (`id`) are NOT accepted by the REST `in_reply_to`
    /// field `ForgeBackend::reply_to_thread`'s GitHub implementation must
    /// post a reply with; that implementation needs the legacy numeric
    /// database ID instead. `None` when the provider's `id` is already
    /// REST-compatible (GitLab discussion/note IDs, Gitea/Forgejo comment
    /// IDs) or simply unavailable. Consumed by
    /// [`crate::model::thread_store::thread_from_remote`], which uses the
    /// root comment's `rest_id` (falling back to `id` when `None`) to seed
    /// the provider's namespaced root-comment ledger at import time — the
    /// same ledger `create_thread` populates — so a thread reply is
    /// possible even for threads this tool never created itself.
    pub rest_id: Option<String>,
}

/// State of a remote review at submit time. GitHub exposes one of
/// `APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`, `DISMISSED`, `PENDING`;
/// we keep the same set so display chrome can mark approvals vs. blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteReviewState {
    Commented,
    Approved,
    ChangesRequested,
    Dismissed,
    Pending,
}

impl RemoteReviewState {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_uppercase().as_str() {
            "APPROVED" => RemoteReviewState::Approved,
            "CHANGES_REQUESTED" => RemoteReviewState::ChangesRequested,
            "DISMISSED" => RemoteReviewState::Dismissed,
            "PENDING" => RemoteReviewState::Pending,
            _ => RemoteReviewState::Commented,
        }
    }

    /// Short label for badge/header text (e.g. `[github @alice approved]`).
    pub fn badge_label(&self) -> Option<&'static str> {
        match self {
            RemoteReviewState::Commented => None,
            RemoteReviewState::Approved => Some("approved"),
            RemoteReviewState::ChangesRequested => Some("changes requested"),
            RemoteReviewState::Dismissed => Some("dismissed"),
            RemoteReviewState::Pending => Some("pending"),
        }
    }
}

/// A review-level summary comment, attached directly to a `PullRequestReview`.
///
/// Distinct from `RemoteReviewThread`: these have no file/line anchor and
/// carry the reviewer's summary text alongside the review state (approved /
/// changes requested / commented). They render in the top-of-diff review
/// area, parallel to local `session.review_comments`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteReviewSummary {
    /// Forge-assigned review node ID.
    pub id: String,
    /// Login/handle of the reviewer, when available.
    pub author: Option<String>,
    /// Markdown body as submitted. Always non-empty by construction —
    /// fetchers drop reviews with empty bodies (e.g. bare approvals).
    pub body: String,
    pub state: RemoteReviewState,
    pub created_at: Option<DateTime<Utc>>,
    /// Permalink to the review on the forge.
    pub url: String,
}

/// A validated inclusive range carried by a remote provider anchor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteReviewRange {
    start: u32,
    end: u32,
}

impl RemoteReviewRange {
    pub fn new(start: u32, end: u32) -> Result<Self, String> {
        if end < start {
            return Err(format!(
                "remote review range end {end} is before start {start}"
            ));
        }
        Ok(Self { start, end })
    }

    pub fn start(&self) -> u32 {
        self.start
    }

    pub fn end(&self) -> u32 {
        self.end
    }
}

#[derive(Deserialize)]
struct RemoteReviewRangeWire {
    start: u32,
    end: u32,
}

impl<'de> Deserialize<'de> for RemoteReviewRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = RemoteReviewRangeWire::deserialize(deserializer)?;
        Self::new(wire.start, wire.end).map_err(serde::de::Error::custom)
    }
}

/// A discussion thread on a forge — one root comment plus zero or more replies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteReviewThread {
    /// Forge-assigned thread node ID.
    pub id: String,
    /// File path the thread anchors to.
    pub path: String,
    /// Anchor line on the chosen side. `None` for fully-outdated threads.
    pub line: Option<u32>,
    pub side: RemoteCommentSide,
    pub is_resolved: bool,
    pub is_outdated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<RemoteReviewRange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_native_anchor: Option<serde_json::Value>,
    /// Root comment first, replies in posted order.
    pub comments: Vec<RemoteReviewComment>,
}

impl RemoteReviewThread {
    /// Per the spec, the default `:comments unresolved` view shows only
    /// threads that are neither resolved nor outdated. `:comments all`
    /// shows everything, and `:comments hide` shows nothing.
    pub fn is_active(&self) -> bool {
        !self.is_resolved && !self.is_outdated
    }

    /// The first comment is the thread root for display purposes.
    pub fn root(&self) -> Option<&RemoteReviewComment> {
        self.comments.first()
    }

    /// Iterator over reply comments (everything after the root).
    pub fn replies(&self) -> impl Iterator<Item = &RemoteReviewComment> {
        self.comments.iter().skip(1)
    }
}

/// User-controlled visibility for remote review comments in PR mode.
///
/// Persisted per-session so visibility survives reopen. Default is
/// `Unresolved` — see the spec section "Existing GitHub Comments".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrCommentsVisibility {
    /// Show only unresolved (and not outdated) threads. Default.
    #[default]
    Unresolved,
    /// Show all fetched threads, with muted styling for resolved/outdated.
    All,
    /// Show nothing.
    Hide,
}

impl PrCommentsVisibility {
    /// Decide whether a thread should appear under this visibility setting.
    /// Returns:
    /// - `Some(false)` — render with normal styling
    /// - `Some(true)` — render with muted styling (resolved or outdated)
    /// - `None`       — do not render
    pub fn render_decision(&self, thread: &RemoteReviewThread) -> Option<bool> {
        match self {
            PrCommentsVisibility::Hide => None,
            PrCommentsVisibility::Unresolved => {
                if thread.is_active() {
                    Some(false)
                } else {
                    None
                }
            }
            PrCommentsVisibility::All => Some(!thread.is_active()),
        }
    }

    /// Short label for use in status bar / footer hints.
    pub fn label(&self) -> &'static str {
        match self {
            PrCommentsVisibility::Unresolved => "unresolved",
            PrCommentsVisibility::All => "all",
            PrCommentsVisibility::Hide => "hidden",
        }
    }
}

/// Filter a list of threads by the active visibility setting. Threads that
/// should not render are dropped; remaining ones keep their flags so the
/// renderer can decide on muted styling per-thread.
pub fn filter_threads(
    threads: &[RemoteReviewThread],
    visibility: PrCommentsVisibility,
) -> Vec<&RemoteReviewThread> {
    threads
        .iter()
        .filter(|t| visibility.render_decision(t).is_some())
        .collect()
}

/// Count the number of rendered lines a thread occupies in the diff view.
/// Used by `App::rebuild_annotations` to push the matching number of
/// annotations so cursor/hit-test math stays in sync with rendering.
///
/// Layout (must match `ui::comment_panel::format_remote_thread_lines`):
/// - 1 header line for the root comment (`╭─ [github @author] L42 ──`)
/// - 1 separator line per reply (`├─ ↳ @author ──`)
/// - 1 body line per `\n`-split line in each comment's body
/// - 1 footer line at the end of the thread (`╰────`)
pub fn thread_display_lines(thread: &RemoteReviewThread) -> usize {
    let mut total = 0;
    for comment in &thread.comments {
        // header (root) or separator (reply) + body lines
        total += 1 + comment.body.split('\n').count();
    }
    // single closing rule for the whole thread
    total += 1;
    total
}

/// The durable-local-state overlay for a `RemoteReviewThread`, computed by
/// [`crate::app::App::remote_thread_overlay`] against the corresponding
/// [`crate::model::thread_store::PersistedThread`] (matched by provider +
/// remote thread ID). `RemoteReviewThread` itself is deliberately a
/// read-only, source-of-truth-on-remote DTO (see this module's top-level
/// doc comment) — this overlay is how a reply/resolve/dismiss made
/// *locally* against the durable thread that mirrors it (see
/// `App::reply_to_thread_at_cursor` / `toggle_thread_resolved_at_cursor` /
/// `dismiss_thread_at_cursor`, reached via `thread_id_for_remote_thread`)
/// becomes visible again in render/annotations/export without mutating or
/// re-fetching the remote DTO itself.
#[derive(Debug, Clone)]
pub struct RemoteThreadOverlay {
    /// Replies present on the durable thread that are *not* already
    /// represented by a provider/comment ID on the remote DTO — i.e.
    /// `ThreadComment`s whose `author.kind != AuthorKind::Remote`, added
    /// locally via the TUI after the thread was imported. Always appended
    /// strictly after the remote-authored root/replies, preserving their
    /// original order (mirrors `merge_remote_thread_into_existing`'s own
    /// local-only-replies-appended-last invariant).
    pub local_only_replies: Vec<crate::model::thread::ThreadComment>,
    /// The durable thread's own [`crate::model::thread::ThreadStatus`] —
    /// may diverge from `RemoteReviewThread::is_resolved`/`is_outdated`
    /// when a local reply/resolve/dismiss/reopen has not yet been
    /// reflected by a subsequent remote re-fetch, or when the status was
    /// driven purely locally (anchor Stale/Ambiguous, or a `Dismissed`
    /// state the remote provider has no equivalent concept for).
    pub local_status: crate::model::thread::ThreadStatus,
}

/// `thread_display_lines(thread)` plus one header/separator line and one
/// body-line-count per `\n`-split line for each of `overlay`'s
/// `local_only_replies` — i.e. the exact number of extra rows
/// `ui::comment_panel::format_remote_thread_lines` appends for the overlay
/// when passed the same `overlay`. `overlay: None` (no durable thread
/// found — e.g. before the initial import ran) is identical to
/// `thread_display_lines(thread)`.
pub fn effective_thread_display_lines(
    thread: &RemoteReviewThread,
    overlay: Option<&RemoteThreadOverlay>,
) -> usize {
    let mut total = thread_display_lines(thread);
    if let Some(overlay) = overlay {
        for reply in &overlay.local_only_replies {
            total += 1 + reply.body.split('\n').count();
        }
    }
    total
}

/// Provider/session-only variant of
/// [`crate::app::App::remote_thread_overlay`] — looks up the durable
/// [`crate::model::thread_store::PersistedThread`] that `remote` was
/// imported into (matched on `(provider, remote.id)`, same pair
/// `import_remote_review_threads`/`thread_from_remote` stamp into
/// `provider_mappings`) directly against a `ReviewSession`, without needing
/// an `App`/`DiffSource`. Exists so non-TUI render paths (export/markdown)
/// that only have `(session, provider)` in scope — not a full `App` — can
/// build the same overlay `App::remote_thread_overlay` would, keeping
/// render and export in lockstep. `None` when there is no provider (not PR
/// mode) or the remote thread hasn't been imported yet; callers must treat
/// that identically to "no local activity" and render the remote DTO
/// unchanged.
pub fn remote_thread_overlay_for_session(
    session: &crate::model::ReviewSession,
    provider: Option<&str>,
    remote: &RemoteReviewThread,
) -> Option<RemoteThreadOverlay> {
    let provider = provider?;
    let persisted = session.find_thread_by_provider(provider, &remote.id)?;
    Some(RemoteThreadOverlay {
        local_only_replies: persisted
            .thread
            .replies()
            .filter(|reply| reply.author.kind != crate::model::thread::AuthorKind::Remote)
            .cloned()
            .collect(),
        local_status: persisted.thread.status(),
    })
}

/// Count the number of rendered lines a review summary occupies in the
/// diff view's review-scope area. Layout must match
/// `ui::comment_panel::format_remote_review_summary_lines`:
/// - 1 header line (`├── [github @author commented] ──`)
/// - 1 body line per `\n`-split line in the summary body
/// - 1 footer line (`╰────`)
pub fn summary_display_lines(summary: &RemoteReviewSummary) -> usize {
    1 + summary.body.split('\n').count() + 1
}

/// Group threads by file path for export grouping. Preserves the input
/// order within each file.
pub fn group_threads_by_path(
    threads: &[RemoteReviewThread],
) -> Vec<(&str, Vec<&RemoteReviewThread>)> {
    let mut groups: Vec<(&str, Vec<&RemoteReviewThread>)> = Vec::new();
    for thread in threads {
        if let Some((_, bucket)) = groups.iter_mut().find(|(p, _)| *p == thread.path.as_str()) {
            bucket.push(thread);
        } else {
            groups.push((thread.path.as_str(), vec![thread]));
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_thread(
        id: &str,
        path: &str,
        line: Option<u32>,
        is_resolved: bool,
        is_outdated: bool,
    ) -> RemoteReviewThread {
        RemoteReviewThread {
            id: id.to_string(),
            path: path.to_string(),
            line,
            side: RemoteCommentSide::Right,
            is_resolved,
            is_outdated,
            range: None,
            provider_native_anchor: None,
            comments: vec![RemoteReviewComment {
                id: format!("{id}-root"),
                author: Some("alice".to_string()),
                body: "Root body".to_string(),
                created_at: None,
                in_reply_to: None,
                url: format!("https://example.com/{id}"),
                rest_id: None,
            }],
        }
    }

    #[test]
    fn should_default_visibility_to_unresolved() {
        // given/when
        let v = PrCommentsVisibility::default();
        // then
        assert_eq!(v, PrCommentsVisibility::Unresolved);
    }

    #[test]
    fn should_show_only_active_threads_when_unresolved() {
        // given
        let v = PrCommentsVisibility::Unresolved;
        let active = make_thread("a", "src/lib.rs", Some(10), false, false);
        let resolved = make_thread("b", "src/lib.rs", Some(20), true, false);
        let outdated = make_thread("c", "src/lib.rs", Some(30), false, true);
        // when/then
        assert_eq!(v.render_decision(&active), Some(false));
        assert_eq!(v.render_decision(&resolved), None);
        assert_eq!(v.render_decision(&outdated), None);
    }

    #[test]
    fn should_show_all_threads_with_muted_for_inactive_when_all() {
        // given
        let v = PrCommentsVisibility::All;
        let active = make_thread("a", "src/lib.rs", Some(10), false, false);
        let resolved = make_thread("b", "src/lib.rs", Some(20), true, false);
        let outdated = make_thread("c", "src/lib.rs", Some(30), false, true);
        // when/then
        assert_eq!(v.render_decision(&active), Some(false));
        assert_eq!(v.render_decision(&resolved), Some(true));
        assert_eq!(v.render_decision(&outdated), Some(true));
    }

    #[test]
    fn should_show_no_threads_when_hidden() {
        // given
        let v = PrCommentsVisibility::Hide;
        let active = make_thread("a", "src/lib.rs", Some(10), false, false);
        // when/then
        assert_eq!(v.render_decision(&active), None);
    }

    #[test]
    fn should_filter_threads_preserving_order() {
        // given
        let threads = vec![
            make_thread("a", "src/lib.rs", Some(10), false, false),
            make_thread("b", "src/lib.rs", Some(20), true, false),
            make_thread("c", "src/main.rs", Some(30), false, false),
        ];
        // when
        let unresolved = filter_threads(&threads, PrCommentsVisibility::Unresolved);
        // then
        assert_eq!(unresolved.len(), 2);
        assert_eq!(unresolved[0].id, "a");
        assert_eq!(unresolved[1].id, "c");
    }

    #[test]
    fn should_round_trip_visibility_via_serde() {
        // given
        let cases = [
            PrCommentsVisibility::Unresolved,
            PrCommentsVisibility::All,
            PrCommentsVisibility::Hide,
        ];
        // when/then
        for c in cases {
            let json = serde_json::to_string(&c).unwrap();
            let back: PrCommentsVisibility = serde_json::from_str(&json).unwrap();
            assert_eq!(back, c);
        }
    }

    #[test]
    fn should_group_threads_by_file_preserving_order() {
        // given
        let threads = vec![
            make_thread("a", "src/lib.rs", Some(10), false, false),
            make_thread("b", "src/main.rs", Some(5), false, false),
            make_thread("c", "src/lib.rs", Some(20), false, false),
        ];
        // when
        let groups = group_threads_by_path(&threads);
        // then
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "src/lib.rs");
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[1].0, "src/main.rs");
        assert_eq!(groups[1].1.len(), 1);
    }

    #[test]
    fn should_parse_remote_comment_side() {
        // given/when/then
        assert_eq!(RemoteCommentSide::parse("LEFT"), RemoteCommentSide::Left);
        assert_eq!(RemoteCommentSide::parse("RIGHT"), RemoteCommentSide::Right);
        assert_eq!(RemoteCommentSide::parse("left"), RemoteCommentSide::Left);
        // unknown defaults to RIGHT (head side) — safer for display
        assert_eq!(RemoteCommentSide::parse(""), RemoteCommentSide::Right);
    }

    #[test]
    fn should_validate_and_round_trip_remote_review_ranges() {
        let range = RemoteReviewRange::new(10, 12).unwrap();
        assert_eq!(range.start(), 10);
        assert_eq!(range.end(), 12);
        assert!(RemoteReviewRange::new(12, 10).is_err());

        let json = serde_json::to_value(&range).unwrap();
        assert_eq!(json, serde_json::json!({"start": 10, "end": 12}));
        assert_eq!(
            serde_json::from_value::<RemoteReviewRange>(json).unwrap(),
            range
        );
        assert!(
            serde_json::from_value::<RemoteReviewRange>(
                serde_json::json!({"start": 12, "end": 10})
            )
            .is_err()
        );
    }

    #[test]
    fn should_omit_absent_range_and_native_anchor_from_thread_json() {
        let thread = make_thread("a", "src/lib.rs", Some(10), false, false);
        let json = serde_json::to_value(&thread).unwrap();
        assert!(json.get("range").is_none());
        assert!(json.get("provider_native_anchor").is_none());
        assert_eq!(
            serde_json::from_value::<RemoteReviewThread>(json).unwrap(),
            thread
        );
    }

    /// Regression test (audit finding: "Add rest_id ... serialization
    /// regression tests") covering both `rest_id` states: `Some` (GitHub's
    /// `databaseId`-backed REST identifier, distinct from the GraphQL node
    /// `id`) and `None` (every other provider, whose bare `id` already is
    /// REST-compatible). A prior regression accidentally dropped this
    /// field's build-time initializer (`RemoteReviewComment.rest_id` not
    /// set) in one of the 5 `ForgeBackend::list_review_threads`
    /// implementations after the Azure merge, so this test pins both the
    /// exact JSON key name and that `None` serializes/deserializes losslessly
    /// rather than silently defaulting to a stale/wrong value on omission.
    #[test]
    fn should_round_trip_rest_id_via_serde_for_both_some_and_none() {
        // given
        let with_rest_id = RemoteReviewComment {
            id: "PRRC_kwABC".to_string(),
            author: Some("alice".to_string()),
            body: "Root body".to_string(),
            created_at: None,
            in_reply_to: None,
            url: "https://github.com/o/r/pull/1#discussion_r1".to_string(),
            rest_id: Some("123456789".to_string()),
        };
        let without_rest_id = RemoteReviewComment {
            rest_id: None,
            ..with_rest_id.clone()
        };

        // when
        let with_json = serde_json::to_value(&with_rest_id).unwrap();
        let without_json = serde_json::to_value(&without_rest_id).unwrap();

        // then: the field is always present (no `skip_serializing_if`), so
        // a stored session artifact is unambiguous about which case it is,
        // and both round-trip back to an equal value.
        assert_eq!(with_json["rest_id"], serde_json::json!("123456789"));
        assert_eq!(without_json["rest_id"], serde_json::Value::Null);
        assert_eq!(
            serde_json::from_value::<RemoteReviewComment>(with_json).unwrap(),
            with_rest_id
        );
        assert_eq!(
            serde_json::from_value::<RemoteReviewComment>(without_json).unwrap(),
            without_rest_id
        );
    }
}
