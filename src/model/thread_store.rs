//! Persistence-facing wrapper around the frozen [`crate::model::thread`]
//! domain module, plus the deterministic legacy `Comment` -> `Thread`
//! migration used when loading a pre-thread (`< "1.4"`) session.
//!
//! [`Thread`] itself is intentionally standalone (see its module docs): it
//! knows nothing about `ReviewSession`, providers, or the legacy `Comment`
//! type. This module is where that vocabulary gets wired up for durable
//! storage without touching the frozen, already-tested contract module.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::comment::{Comment, DEFAULT_AUTHOR, LineSide};
use super::thread::{
    Anchor, AnchorSide, CommentId, ProviderRemap, Thread, ThreadAnchorRefresh, ThreadAuthor,
    ThreadComment, ThreadId,
};
use crate::error::Result;

/// A [`Thread`] plus its provider-native ID/opaque-payload mappings, keyed
/// by provider name (e.g. `"github"`, `"gitlab"`). Mirrors
/// `schemas/review-artifact-v1.schema.json#/$defs/thread.provider_mappings`.
///
/// No credentials are ever stored here — only whatever opaque JSON payload
/// a provider adapter supplies (e.g. a native thread/comment ID and
/// anchor). [`Self::upsert_provider_mapping`] overwrites the existing entry
/// for a given provider rather than accumulating duplicates, so repeated
/// import/sync passes for the same remote thread are idempotent by
/// construction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedThread {
    #[serde(flatten)]
    pub thread: Thread,
    /// Opaque, per-provider mapping payload. Callers are expected to store
    /// at least an `"id"` string field so [`Self::has_provider_id`] and
    /// [`super::review::ReviewSession::find_thread_by_provider`] can look
    /// threads up idempotently by provider-native ID, but this module does
    /// not enforce any particular shape beyond "valid JSON object".
    #[serde(default)]
    pub provider_mappings: HashMap<String, serde_json::Value>,
}

impl PersistedThread {
    pub fn new(thread: Thread) -> Self {
        Self {
            thread,
            provider_mappings: HashMap::new(),
        }
    }

    pub fn id(&self) -> &ThreadId {
        self.thread.id()
    }

    /// Insert or overwrite this thread's mapping for `provider`. Calling
    /// this repeatedly with the same `provider` key is idempotent: the
    /// previous payload is replaced, never duplicated.
    pub fn upsert_provider_mapping(
        &mut self,
        provider: impl Into<String>,
        mapping: serde_json::Value,
    ) {
        self.provider_mappings.insert(provider.into(), mapping);
    }

    pub fn provider_mapping(&self, provider: &str) -> Option<&serde_json::Value> {
        self.provider_mappings.get(provider)
    }

    /// True if this thread's `provider` mapping carries the given
    /// provider-native `id` (read from a conventional `"id"` string field
    /// in the opaque payload).
    pub fn has_provider_id(&self, provider: &str, id: &str) -> bool {
        self.provider_mapping(provider)
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str())
            == Some(id)
    }

    /// Re-evaluate this thread's anchor against updated file content,
    /// letting an exact provider remap win over unique context relocation.
    /// Thin pass-through to [`Thread::refresh_anchor_with_remap`] kept here
    /// so store callers only need to import [`PersistedThread`].
    pub fn refresh_anchor_with_remap(
        &mut self,
        new_lines: &[&str],
        provider_remap: Option<&ProviderRemap>,
    ) -> Result<ThreadAnchorRefresh> {
        self.thread
            .refresh_anchor_with_remap(new_lines, provider_remap)
    }
}

/// Current session schema version. Sessions below this version have their
/// legacy `Comment`s migrated into [`PersistedThread`]s the first time they
/// are loaded (see [`super::review::ReviewSession::migrate_legacy_comments_to_threads`]).
pub const CURRENT_SESSION_VERSION: &str = "1.4";

/// Construct a [`CommentId`] whose serialized value is exactly
/// `raw_id` (e.g. a legacy `Comment.id`), rather than a freshly minted
/// random one.
///
/// `CommentId`'s inner string is private outside `thread.rs`, and its only
/// public constructor (`CommentId::new`) always mints a random UUID v4.
/// Since `CommentId` derives `Deserialize` as a plain newtype (serde treats
/// single-field tuple structs as transparent), round-tripping a chosen JSON
/// string through `Deserialize` is the only way to reuse an existing ID
/// without modifying the frozen `thread.rs` module.
fn comment_id_from_raw(raw_id: &str) -> CommentId {
    serde_json::from_value(serde_json::Value::String(raw_id.to_string()))
        .expect("CommentId deserializes transparently from any string")
}

/// Construct a [`ThreadId`] whose serialized value is exactly `raw_id`, via
/// the same transparent-`Deserialize` technique as [`comment_id_from_raw`].
fn thread_id_from_raw(raw_id: &str) -> ThreadId {
    serde_json::from_value(serde_json::Value::String(raw_id.to_string()))
        .expect("ThreadId deserializes transparently from any string")
}

/// Deterministically derive a migrated thread's [`ThreadId`] from its
/// anchor target and its root comment's (preserved) [`CommentId`].
///
/// Using the anchor target (rather than only the comment ID) means two
/// otherwise-identical legacy comment IDs anchored at different targets
/// (which cannot happen in practice, since legacy `Comment.id`s are already
/// globally unique, but is defensive against any future relaxation of that
/// invariant) still cannot collide. The anchor target is serialized to a
/// canonical JSON string via the frozen module's own `Serialize` impl, so
/// this derivation never needs to inspect `Anchor`'s private fields.
fn deterministic_thread_id(anchor: &Anchor, root_id: &CommentId) -> ThreadId {
    let anchor_fingerprint =
        serde_json::to_string(anchor.target()).expect("AnchorTarget always serializes");
    thread_id_from_raw(&format!(
        "legacy-thread:{anchor_fingerprint}:{}",
        root_id.as_str()
    ))
}

/// Build a [`ThreadComment`] whose `id` is exactly `id` (rather than a
/// freshly minted random [`CommentId`]), via a JSON round-trip through the
/// frozen module's own derived `Deserialize` impl. `author`/`body`/
/// `created_at` are set as given; `updated_at` starts `None`, matching
/// [`ThreadComment::new`]'s defaults.
fn thread_comment_with_id(
    id: CommentId,
    author: ThreadAuthor,
    body: String,
    created_at: chrono::DateTime<chrono::Utc>,
) -> ThreadComment {
    let value = serde_json::json!({
        "id": id,
        "author": author,
        "body": body,
        "created_at": created_at,
        "updated_at": Option::<chrono::DateTime<chrono::Utc>>::None,
    });
    serde_json::from_value(value).expect("ThreadComment fields always round-trip through JSON")
}

/// Build a [`Thread`] whose `id` is exactly `id` (rather than a freshly
/// minted random [`ThreadId`] from [`Thread::open`]), via the same
/// JSON-round-trip technique as [`thread_comment_with_id`]. Always starts
/// `Open` with a single root comment, matching `Thread::open`'s invariants.
fn thread_with_id(id: ThreadId, anchor: Anchor, root: ThreadComment) -> Thread {
    let value = serde_json::json!({
        "id": id,
        "status": "open",
        "anchor": anchor,
        "comments": [root],
    });
    serde_json::from_value(value).expect("Thread fields always round-trip through JSON")
}

/// Build a one-comment [`PersistedThread`] from a legacy `Comment`,
/// preserving its original `content`, `author` name, and `created_at`.
///
/// Legacy `Comment` has no explicit human/agent distinction — only a
/// free-form `author` string (humans default to `Comment::DEFAULT_AUTHOR`
/// ("user"); agents pass an explicit `--username`). This migration uses
/// that convention as a heuristic: the default author name migrates as
/// [`ThreadAuthor::human`], any other author name migrates as
/// [`ThreadAuthor::agent`]. The original name string is always preserved
/// verbatim either way, so authorship is never lost even when the kind
/// guess is imperfect.
///
/// IDs are deterministic, not randomly minted: the root [`ThreadComment`]
/// reuses the legacy `Comment.id` verbatim as its [`CommentId`], and the
/// [`Thread`]'s [`ThreadId`] is derived from the anchor target plus that
/// same comment ID (see [`deterministic_thread_id`]). Migrating the same
/// legacy session twice therefore produces byte-identical thread/comment
/// identities (never a fresh random ID, never a duplicate thread), which is
/// what makes `migrate_legacy_comments_to_threads` safe to run on every
/// session load rather than only once.
pub(super) fn thread_from_legacy_comment(anchor: Anchor, comment: &Comment) -> PersistedThread {
    let author = legacy_comment_author(comment);
    let root_id = comment_id_from_raw(&comment.id);
    let thread_id = deterministic_thread_id(&anchor, &root_id);
    let root = thread_comment_with_id(root_id, author, comment.content.clone(), comment.created_at);
    PersistedThread::new(thread_with_id(thread_id, anchor, root))
}

/// Migrate a *group* of legacy `Comment`s that all resolve to the exact
/// same `Anchor` target (e.g. several independent notes left on the same
/// diff line/range, which the legacy model stores as a flat
/// `Vec<Comment>` with no root/reply distinction) into a single
/// [`Thread`]: the first comment in original insertion order becomes the
/// root, and every following comment becomes an ordered reply. This
/// mirrors the frozen model's own framing of a thread as "a discussion
/// anchored at one place" rather than fragmenting one anchor point into
/// several one-comment threads.
///
/// Every comment -- root or reply -- keeps its own legacy `Comment.id` as
/// its `CommentId` verbatim (see [`thread_from_legacy_comment`]), and the
/// resulting `ThreadId` is still derived only from the anchor plus the
/// *root's* id, so re-migrating the same group twice is still fully
/// deterministic and produces byte-identical thread/comment/reply
/// identities and ordering.
///
/// # Panics
/// `comments` must be non-empty; callers only invoke this for an anchor
/// that has at least one legacy comment.
pub(super) fn thread_from_legacy_comment_group(
    anchor: Anchor,
    comments: &[Comment],
) -> PersistedThread {
    let (root_comment, replies) = comments
        .split_first()
        .expect("thread_from_legacy_comment_group requires a non-empty comment group");
    let mut persisted = thread_from_legacy_comment(anchor, root_comment);
    for comment in replies {
        let reply_id = comment_id_from_raw(&comment.id);
        let reply = thread_comment_with_id(
            reply_id,
            legacy_comment_author(comment),
            comment.content.clone(),
            comment.created_at,
        );
        persisted.thread.reply(reply);
    }
    persisted
}

/// Convert a legacy `Comment`'s free-form `author` string into the frozen
/// module's [`ThreadAuthor`] vocabulary: the sentinel [`DEFAULT_AUTHOR`]
/// maps to [`ThreadAuthor::human`], any other value to
/// [`ThreadAuthor::agent`]. Extracted so both
/// [`thread_from_legacy_comment`] and
/// [`thread_from_legacy_comment_group`]'s reply path apply the exact same
/// rule.
fn legacy_comment_author(comment: &Comment) -> ThreadAuthor {
    if comment.author == DEFAULT_AUTHOR {
        ThreadAuthor::human(comment.author.clone())
    } else {
        ThreadAuthor::agent(comment.author.clone())
    }
}

/// Map a legacy `Comment`'s `side` to the frozen module's [`AnchorSide`].
/// Legacy comments never express `Both`.
pub(super) fn anchor_side_for_legacy(side: Option<LineSide>) -> AnchorSide {
    match side.unwrap_or_default() {
        LineSide::Old => AnchorSide::Old,
        LineSide::New => AnchorSide::New,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::comment::CommentType;
    use crate::model::thread::AuthorKind;

    fn legacy_comment(author: &str, content: &str) -> Comment {
        Comment::new(content.to_string(), CommentType::None, None).with_author(author)
    }

    #[test]
    fn should_preserve_legacy_comment_id_as_thread_comment_id() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let legacy_id = comment.id.clone();
        let persisted = thread_from_legacy_comment(Anchor::review(), &comment);
        assert_eq!(
            persisted.thread.root().unwrap().id().as_str(),
            legacy_id,
            "root ThreadComment id must reuse the legacy Comment.id verbatim, not a fresh random id"
        );
    }

    #[test]
    fn should_derive_the_same_thread_id_across_repeat_migrations_of_the_same_comment() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");

        let first = thread_from_legacy_comment(Anchor::line("a.rs", AnchorSide::New, 10), &comment);
        let second =
            thread_from_legacy_comment(Anchor::line("a.rs", AnchorSide::New, 10), &comment);

        assert_eq!(
            first.id(),
            second.id(),
            "migrating the same legacy comment/anchor twice must derive an identical ThreadId, never a random one"
        );
        assert_eq!(
            first.thread.root().unwrap().id(),
            second.thread.root().unwrap().id()
        );
    }

    #[test]
    fn should_derive_different_thread_ids_for_different_anchors_of_the_same_comment_id() {
        // Defensive: even if two comments somehow shared an id, differing
        // anchors must not collapse to the same ThreadId.
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");

        let at_line_10 =
            thread_from_legacy_comment(Anchor::line("a.rs", AnchorSide::New, 10), &comment);
        let at_line_11 =
            thread_from_legacy_comment(Anchor::line("a.rs", AnchorSide::New, 11), &comment);

        assert_ne!(at_line_10.id(), at_line_11.id());
    }

    #[test]
    fn should_migrate_default_author_as_human() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "looks good");
        let persisted = thread_from_legacy_comment(Anchor::review(), &comment);
        let root = persisted.thread.root().unwrap();
        assert_eq!(root.author.kind, AuthorKind::Human);
        assert_eq!(root.author.name, DEFAULT_AUTHOR);
        assert_eq!(root.body, "looks good");
    }

    #[test]
    fn should_migrate_custom_author_as_agent() {
        let comment = legacy_comment("Claude Opus", "consider a null check");
        let persisted = thread_from_legacy_comment(Anchor::review(), &comment);
        let root = persisted.thread.root().unwrap();
        assert_eq!(root.author.kind, AuthorKind::Agent);
        assert_eq!(root.author.name, "Claude Opus");
    }

    #[test]
    fn should_preserve_original_created_at() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let original_created_at = comment.created_at;
        let persisted = thread_from_legacy_comment(Anchor::review(), &comment);
        assert_eq!(
            persisted.thread.root().unwrap().created_at,
            original_created_at
        );
    }

    #[test]
    fn should_upsert_provider_mapping_idempotently() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);

        persisted.upsert_provider_mapping("github", serde_json::json!({"id": "abc"}));
        persisted.upsert_provider_mapping("github", serde_json::json!({"id": "abc"}));

        assert_eq!(persisted.provider_mappings.len(), 1);
        assert!(persisted.has_provider_id("github", "abc"));
        assert!(!persisted.has_provider_id("github", "other"));
        assert!(!persisted.has_provider_id("gitlab", "abc"));
    }

    #[test]
    fn should_overwrite_provider_mapping_on_repeat_import_with_new_payload() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);

        persisted.upsert_provider_mapping("github", serde_json::json!({"id": "abc", "line": 1}));
        persisted.upsert_provider_mapping("github", serde_json::json!({"id": "abc", "line": 2}));

        assert_eq!(persisted.provider_mappings.len(), 1);
        assert_eq!(
            persisted.provider_mapping("github").unwrap()["line"],
            serde_json::json!(2)
        );
    }

    #[test]
    fn should_roundtrip_persisted_thread_json_with_flattened_thread_fields() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);
        persisted.upsert_provider_mapping("github", serde_json::json!({"id": "abc"}));

        let json = serde_json::to_value(&persisted).unwrap();
        assert_eq!(json["status"], serde_json::json!("open"));
        assert!(
            json.get("thread").is_none(),
            "thread fields must be flattened, not nested"
        );

        let restored: PersistedThread = serde_json::from_value(json).unwrap();
        assert_eq!(restored, persisted);
    }

    #[test]
    fn should_map_legacy_line_sides_without_both() {
        assert_eq!(anchor_side_for_legacy(Some(LineSide::Old)), AnchorSide::Old);
        assert_eq!(anchor_side_for_legacy(Some(LineSide::New)), AnchorSide::New);
        assert_eq!(anchor_side_for_legacy(None), AnchorSide::New);
    }
}
