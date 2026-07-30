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
    Anchor, AnchorSide, ProviderRemap, Thread, ThreadAnchorRefresh, ThreadAuthor, ThreadComment,
    ThreadId,
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
pub(super) fn thread_from_legacy_comment(anchor: Anchor, comment: &Comment) -> PersistedThread {
    let author = if comment.author == DEFAULT_AUTHOR {
        ThreadAuthor::human(comment.author.clone())
    } else {
        ThreadAuthor::agent(comment.author.clone())
    };
    let mut root = ThreadComment::new(author, comment.content.clone());
    root.created_at = comment.created_at;
    PersistedThread::new(Thread::open(anchor, root))
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
