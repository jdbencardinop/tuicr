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
    Anchor, AnchorSide, AuthorKind, CommentId, ProviderRemap, Thread, ThreadAnchorRefresh,
    ThreadAuthor, ThreadComment, ThreadId, ThreadStatus,
};
use crate::error::Result;
use crate::forge::remote_comments::{RemoteCommentSide, RemoteReviewThread};

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

    /// Legacy migrations use a stable, namespaced ID rather than a random
    /// thread UUID. The prefix remains valid after anchor relocation.
    pub fn is_legacy_mirror(&self) -> bool {
        self.id().as_str().starts_with("legacy-thread:")
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

    /// Record that this thread's root comment was published to `provider`
    /// as `provider_comment_id`. Stored under a namespaced key (see
    /// [`root_comment_mapping_key`]) distinct from the thread's own bare
    /// `provider` mapping, so it survives a subsequent remote re-import
    /// (see that function's doc comment for why). Idempotent: recording
    /// again for the same provider overwrites rather than duplicates.
    pub fn record_root_comment_id(
        &mut self,
        provider: &str,
        provider_comment_id: impl Into<String>,
    ) {
        self.upsert_provider_mapping(
            root_comment_mapping_key(provider),
            serde_json::Value::String(provider_comment_id.into()),
        );
    }

    /// Look up this thread's previously recorded provider-native root
    /// -comment ID for `provider`, from the namespaced ledger written by
    /// [`Self::record_root_comment_id`]. Falls back to the bare mapping's
    /// own `"root_comment_id"` field (present right after `create_thread`,
    /// before any re-import could have wiped it) so callers get a value
    /// even before the ledger entry has been written.
    pub fn root_comment_id(&self, provider: &str) -> Option<&str> {
        self.provider_mapping(&root_comment_mapping_key(provider))
            .and_then(|value| value.as_str())
            .or_else(|| {
                self.provider_mapping(provider)
                    .and_then(|value| value.get("root_comment_id"))
                    .and_then(|value| value.as_str())
            })
    }

    /// Look up a previously recorded provider-native reply/note ID for the
    /// local `comment_id`, stored under [`replies_mapping_key`]. Returns
    /// `None` when the reply has never been published to `provider` (or the
    /// thread has no reply ledger yet at all).
    pub fn published_reply_id(&self, provider: &str, comment_id: &str) -> Option<&str> {
        self.provider_mapping(&replies_mapping_key(provider))
            .and_then(|value| value.get(comment_id))
            .and_then(|value| value.as_str())
    }

    /// Record that the local reply `comment_id` was successfully published
    /// to `provider` as `provider_comment_id`. Stored under a namespaced key
    /// (see [`replies_mapping_key`]) distinct from the thread's own
    /// `provider` mapping, so a subsequent remote re-import/merge (which
    /// wholesale-replaces the bare `provider` entry via
    /// [`merge_remote_thread_into_existing`]) never clobbers this ledger.
    /// Idempotent: publishing the same `comment_id` again overwrites rather
    /// than duplicates the entry.
    pub fn record_published_reply(
        &mut self,
        provider: &str,
        comment_id: impl Into<String>,
        provider_comment_id: impl Into<String>,
    ) {
        let key = replies_mapping_key(provider);
        let mut replies = self
            .provider_mapping(&key)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        if !replies.is_object() {
            replies = serde_json::json!({});
        }
        replies[comment_id.into()] = serde_json::Value::String(provider_comment_id.into());
        self.upsert_provider_mapping(key, replies);
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
        persisted.thread.reply(legacy_reply_comment(comment));
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

/// Compute the deterministic [`ThreadId`] that migrating a legacy comment
/// (or comment group) anchored at `anchor` with `root_comment_id` as its
/// root would produce, without constructing a full [`PersistedThread`].
///
/// Lets [`super::review::ReviewSession::migrate_legacy_comments_to_threads`]
/// check whether a thread for this exact anchor/root combination already
/// exists *before* deciding whether to create a brand-new thread or append
/// incremental replies to one already synced from a previous load -- this
/// is what makes re-running migration on an already-`CURRENT_SESSION_VERSION`
/// session (e.g. after a plain `review add` appended one more legacy
/// comment) idempotent instead of a version-gated no-op that permanently
/// stops mirroring new legacy comments into threads.
pub(super) fn legacy_thread_id_for(anchor: &Anchor, root_comment_id: &str) -> ThreadId {
    deterministic_thread_id(anchor, &comment_id_from_raw(root_comment_id))
}

/// Build a [`ThreadComment`] reply from a legacy `Comment`, preserving its
/// id/author/body/created_at -- the same conversion
/// [`thread_from_legacy_comment_group`] applies to every non-root comment
/// in a group, extracted so incremental catch-up syncing (appending a
/// newly-added legacy comment to an already-migrated thread) can build the
/// exact same reply shape without re-deriving a `ThreadId` or wrapping it
/// in a new [`Thread`]/[`PersistedThread`].
pub(super) fn legacy_reply_comment(comment: &Comment) -> ThreadComment {
    thread_comment_with_id(
        comment_id_from_raw(&comment.id),
        legacy_comment_author(comment),
        comment.content.clone(),
        comment.created_at,
    )
}

/// Map a legacy `Comment`'s `side` to the frozen module's [`AnchorSide`].
/// Legacy comments never express `Both`.
pub(super) fn anchor_side_for_legacy(side: Option<LineSide>) -> AnchorSide {
    match side.unwrap_or_default() {
        LineSide::Old => AnchorSide::Old,
        LineSide::New => AnchorSide::New,
    }
}

/// Map a fetched [`RemoteCommentSide`] to the frozen module's [`AnchorSide`].
/// Remote threads never express `Both` either.
fn anchor_side_for_remote(side: RemoteCommentSide) -> AnchorSide {
    match side {
        RemoteCommentSide::Left => AnchorSide::Old,
        RemoteCommentSide::Right => AnchorSide::New,
    }
}

/// Deterministically derive the [`ThreadId`] that importing `remote` from
/// `provider` would produce, without constructing a full [`PersistedThread`].
///
/// Keyed on `(provider, remote.id)` rather than the anchor, unlike
/// [`deterministic_thread_id`]: a provider-native thread ID is already
/// globally unique and stable across re-fetches (unlike a legacy
/// `Comment`, a remote thread's line/path can themselves change as the PR
/// head advances without the provider minting a new thread ID), so basing
/// the derivation on it — rather than the anchor — is what keeps a single
/// remote thread mapped onto the same durable [`PersistedThread`] even
/// after its anchor relocates.
pub fn remote_thread_id_for(provider: &str, remote_id: &str) -> ThreadId {
    thread_id_from_raw(&format!("remote-thread:{provider}:{remote_id}"))
}

/// The `provider_mappings` key used to track published-reply IDs for
/// `provider`, namespaced away from `provider`'s own bare key (see
/// [`PersistedThread::record_published_reply`]/
/// [`PersistedThread::published_reply_id`]).
pub fn replies_mapping_key(provider: &str) -> String {
    format!("{provider}:replies")
}

/// The `provider_mappings` key used to durably track a thread's own
/// root-comment ID for `provider` (see
/// [`PersistedThread::record_root_comment_id`]/
/// [`PersistedThread::root_comment_id`]), namespaced away from the bare
/// `provider` key for the same reason as [`replies_mapping_key`]: some
/// backends (GitHub) embed `root_comment_id` inside the bare mapping too
/// (as a convenience for readers that already have it in hand right after
/// `create_thread`), but that copy does not survive
/// [`merge_remote_thread_into_existing`]'s wholesale replacement of the
/// bare key on the next remote re-import/sync. This namespaced copy does,
/// so [`crate::forge::traits::ForgeBackend::reply_to_thread`] callers can
/// still find the root comment to reply to after a thread has been
/// re-imported since it was created.
pub fn root_comment_mapping_key(provider: &str) -> String {
    format!("{provider}:root")
}

/// Convert a fetched [`RemoteReviewThread`] into a [`PersistedThread`],
/// preserving root/reply order, comment IDs, authors (as
/// [`ThreadAuthor::remote`]), and the provider's resolved/outdated state.
///
/// Idempotent: the resulting [`Thread`]'s ID is derived solely from
/// `(provider, remote.id)` (see [`remote_thread_id_for`]), and every
/// comment/reply reuses the remote comment's own ID verbatim (via the same
/// transparent-`Deserialize` technique as [`thread_from_legacy_comment`]),
/// so converting the same remote thread twice always produces
/// byte-identical IDs — callers (see
/// [`super::review::ReviewSession::import_remote_review_threads`]) use
/// this to replace rather than duplicate an already-imported thread.
///
/// `remote.line` is `None` for fully-outdated threads with no current
/// anchor line; those import as a file-level [`Anchor::file`] rather than a
/// line anchor, since a line anchor requires a concrete line number.
///
/// # Panics
/// `remote.comments` must be non-empty (a thread always has a root); only
/// call this after checking [`RemoteReviewThread::root`] is `Some`.
pub fn thread_from_remote(provider: &str, remote: &RemoteReviewThread) -> PersistedThread {
    let (root_comment, replies) = remote
        .comments
        .split_first()
        .expect("thread_from_remote requires a non-empty RemoteReviewThread.comments");

    let side = anchor_side_for_remote(remote.side);
    let anchor = if let Some(range) = remote.range.as_ref() {
        Anchor::range(remote.path.clone(), side, range.start(), range.end())
            .expect("RemoteReviewRange guarantees ordered inclusive bounds")
    } else {
        match remote.line {
            Some(line) => Anchor::line(remote.path.clone(), side, line),
            None => Anchor::file(remote.path.clone()),
        }
    };

    let thread_id = remote_thread_id_for(provider, &remote.id);
    let root = remote_thread_comment(root_comment);
    let mut thread = thread_with_id(thread_id, anchor, root);
    for reply in replies {
        thread.reply(remote_thread_comment(reply));
    }
    if remote.is_outdated {
        thread.mark_stale_from_provider();
    }
    if remote.is_resolved {
        thread.resolve();
    }

    let mut persisted = PersistedThread::new(thread);
    let mut provider_mapping = serde_json::json!({
        "id": remote.id,
        "path": remote.path,
        "line": remote.line,
        "is_outdated": remote.is_outdated,
        "is_resolved": remote.is_resolved,
    });
    if let Some(range) = remote.range.as_ref() {
        provider_mapping["range"] = serde_json::json!({
            "start": range.start(),
            "end": range.end(),
        });
    }
    if let Some(native_anchor) = remote.provider_native_anchor.as_ref() {
        provider_mapping["native_anchor"] = native_anchor.clone();
    }
    persisted.upsert_provider_mapping(provider, provider_mapping);
    // Seed the namespaced root-comment ledger from the remote payload
    // itself, the same ledger `create_thread` populates via
    // `record_root_comment_id`, so `ForgeBackend::reply_to_thread` can
    // reply to a thread this tool never created (only ever imported).
    // Only providers whose reply mutation needs an ID distinct from the
    // thread-lookup `id` populate `rest_id` (currently: GitHub, via
    // `databaseId`); other providers' bare `id` is already what
    // `reply_to_thread` needs, so there is nothing to seed for them.
    if let Some(rest_id) = &root_comment.rest_id {
        persisted.record_root_comment_id(provider, rest_id.clone());
    }
    persisted
}

/// Merge a freshly re-converted [`thread_from_remote`] result (`fresh`)
/// into `existing`'s already-imported copy of the same remote thread
/// (matched by `existing.id() == fresh.id()`), used by
/// [`super::review::ReviewSession::import_remote_review_threads`] instead
/// of a naive full overwrite on re-import.
///
/// Re-fetching an unchanged remote thread twice (the finding-5 idempotency
/// requirement) is a strict subset of this: `fresh` then carries the exact
/// same remote-authored comments/resolved-flag `existing` already holds,
/// so the merge is a no-op and produces a byte-identical thread.
///
/// What changes on a genuine re-fetch is merged as follows:
/// - **Anchor**: left completely untouched. This function's caller is not
///   the anchor's owner — [`Thread::refresh_anchor`]/
///   [`Thread::refresh_anchor_with_remap`] (driven by the PR-head-advance
///   path) is — so a background remote re-fetch never silently resets a
///   locally computed Stale/Ambiguous relocation state back to a naive
///   "current" anchor built fresh from the remote payload's raw path/line.
/// - **Comments**: the root and every *remote-authored* reply are taken
///   from `fresh` (the provider's current truth for its own content,
///   including any edit, or a reply added/removed directly on the
///   provider since the last fetch); any *locally*-authored (`Human`/
///   `Agent`) reply already on `existing` (added via a TUI thread-reply
///   action after the thread was first imported) is preserved, appended
///   after the remote-authored ones so remote root/reply order stays
///   intact and local additions are never lost.
/// - **Status**: never regresses. Once `existing` is anything other than
///   `Open` (locally `Resolved`/`Dismissed`, or anchor-driven `Stale`/
///   `Ambiguous`), a stale or not-yet-caught-up remote fetch never
///   silently reopens or overwrites it — only a forward transition (still
///   locally `Open`, remote now reports resolved) is ever applied. This
///   means a local Stale/Ambiguous thread whose remote counterpart is
///   independently resolved on the provider does *not* flip to
///   `Resolved` in `status`; the provider's resolved/outdated flags are
///   still captured verbatim in `provider_mappings` below either way, so
///   that information is never lost, just not allowed to overwrite the
///   local anchor-relocation signal.
/// - **`provider_mappings`**: merged, not replaced wholesale — `fresh`'s
///   single provider entry is upserted (so its `is_resolved`/`is_outdated`
///   native flags always reflect the latest fetch), while any other
///   provider's mapping already on `existing` (e.g. the same thread also
///   published/imported on a different provider) is preserved.
pub(super) fn merge_remote_thread_into_existing(
    existing: &mut PersistedThread,
    fresh: PersistedThread,
    provider: &str,
) {
    let fresh_anchor_is_stale = fresh.thread.anchor().is_stale();
    let local_only_replies: Vec<ThreadComment> = existing
        .thread
        .replies()
        .filter(|reply| reply.author.kind != AuthorKind::Remote)
        .cloned()
        .collect();

    let mut comments: Vec<ThreadComment> = Vec::new();
    comments.push(
        fresh
            .thread
            .root()
            .cloned()
            .expect("thread_from_remote always produces a thread with a root comment"),
    );
    comments.extend(fresh.thread.replies().cloned());
    comments.extend(local_only_replies);

    let status = if existing.thread.status() == ThreadStatus::Open {
        fresh.thread.status()
    } else {
        existing.thread.status()
    };

    let merged_thread_value = serde_json::json!({
        "id": existing.id(),
        "status": status,
        "anchor": existing.thread.anchor(),
        "comments": comments,
    });
    existing.thread = serde_json::from_value(merged_thread_value)
        .expect("Thread fields always round-trip through JSON");
    if fresh_anchor_is_stale {
        existing.thread.mark_stale_from_provider();
    }

    if let Some(mapping) = fresh.provider_mapping(provider) {
        existing.upsert_provider_mapping(provider, mapping.clone());
    }
}

/// Build a [`ThreadComment`] from a fetched [`RemoteReviewComment`],
/// preserving its ID verbatim and tagging its author
/// [`ThreadAuthor::remote`] (the provider payload does not distinguish
/// human vs. bot authors, so every remote import uses this one kind,
/// unlike legacy migration's human/agent heuristic).
fn remote_thread_comment(
    comment: &crate::forge::remote_comments::RemoteReviewComment,
) -> ThreadComment {
    let author_name = comment
        .author
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    thread_comment_with_id(
        comment_id_from_raw(&comment.id),
        ThreadAuthor::remote(author_name.clone(), author_name),
        comment.body.clone(),
        comment.created_at.unwrap_or_else(chrono::Utc::now),
    )
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

    #[test]
    fn should_record_and_look_up_published_reply_ids_idempotently() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);

        assert_eq!(persisted.published_reply_id("github", "local-1"), None);

        persisted.record_published_reply("github", "local-1", "PRRC_1");
        persisted.record_published_reply("github", "local-1", "PRRC_1");

        assert_eq!(
            persisted.published_reply_id("github", "local-1"),
            Some("PRRC_1")
        );
        // A second, distinct reply gets its own entry without disturbing the
        // first.
        persisted.record_published_reply("github", "local-2", "PRRC_2");
        assert_eq!(
            persisted.published_reply_id("github", "local-1"),
            Some("PRRC_1")
        );
        assert_eq!(
            persisted.published_reply_id("github", "local-2"),
            Some("PRRC_2")
        );
        // The reply ledger lives under a namespaced key, distinct from the
        // provider's own bare mapping.
        assert!(persisted.provider_mapping("github").is_none());
        assert!(
            persisted
                .provider_mapping(&replies_mapping_key("github"))
                .is_some()
        );
    }

    #[test]
    fn should_keep_reply_ledgers_isolated_per_provider() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);

        persisted.record_published_reply("github", "local-1", "PRRC_1");
        persisted.record_published_reply("gitlab", "local-1", "note-9");

        assert_eq!(
            persisted.published_reply_id("github", "local-1"),
            Some("PRRC_1")
        );
        assert_eq!(
            persisted.published_reply_id("gitlab", "local-1"),
            Some("note-9")
        );
    }

    #[test]
    fn should_survive_remote_merge_overwriting_the_bare_provider_mapping() {
        // A reply ledger recorded under the namespaced key must not be
        // clobbered when a subsequent remote re-import replaces the bare
        // `provider` mapping wholesale (see `merge_remote_thread_into_existing`).
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);
        persisted.record_published_reply("github", "local-1", "PRRC_1");
        persisted.upsert_provider_mapping("github", serde_json::json!({"id": "PRRT_1"}));

        // Simulate the bare-key replacement `merge_remote_thread_into_existing`
        // performs on every re-fetch.
        persisted.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "is_resolved": true}),
        );

        assert_eq!(
            persisted.published_reply_id("github", "local-1"),
            Some("PRRC_1")
        );
    }

    #[test]
    fn should_record_and_look_up_root_comment_id_idempotently() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);

        assert_eq!(persisted.root_comment_id("github"), None);

        persisted.record_root_comment_id("github", "PRRC_1");
        persisted.record_root_comment_id("github", "PRRC_1");
        assert_eq!(persisted.root_comment_id("github"), Some("PRRC_1"));

        // Recording again overwrites rather than duplicating/erroring.
        persisted.record_root_comment_id("github", "PRRC_2");
        assert_eq!(persisted.root_comment_id("github"), Some("PRRC_2"));
    }

    #[test]
    fn should_survive_remote_merge_overwriting_the_bare_provider_mapping_for_root_comment_id() {
        // Mirrors the reply-ledger survival test above: the root-comment-id
        // ledger must not be clobbered when a remote re-import replaces the
        // bare `provider` mapping wholesale.
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);
        persisted.record_root_comment_id("github", "PRRC_1");
        persisted.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "root_comment_id": "PRRC_1"}),
        );

        // Simulate the bare-key replacement `merge_remote_thread_into_existing`
        // performs on every re-fetch — the bare mapping's own
        // `root_comment_id` copy is gone, but the namespaced ledger survives.
        persisted.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "is_resolved": true}),
        );

        assert_eq!(persisted.root_comment_id("github"), Some("PRRC_1"));
    }

    #[test]
    fn should_fall_back_to_the_bare_mapping_root_comment_id_before_the_ledger_is_written() {
        // Right after `create_thread` succeeds but before the caller has
        // called `record_root_comment_id`, the bare mapping's own
        // `root_comment_id` field (written by `build_create_thread_response`)
        // must still resolve.
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);
        persisted.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "root_comment_id": "PRRC_1"}),
        );

        assert_eq!(persisted.root_comment_id("github"), Some("PRRC_1"));
    }

    fn remote_thread_with_rest_id(rest_id: Option<&str>) -> RemoteReviewThread {
        RemoteReviewThread {
            id: "PRRT_1".to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(10),
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            range: None,
            provider_native_anchor: None,
            comments: vec![crate::forge::remote_comments::RemoteReviewComment {
                id: "PRRC_kwABC".to_string(),
                author: Some("alice".to_string()),
                body: "root".to_string(),
                created_at: None,
                in_reply_to: None,
                url: "https://github.com/o/r/pull/1#discussion_r1".to_string(),
                rest_id: rest_id.map(str::to_string),
            }],
        }
    }

    /// Regression test (audit finding: "Add rest_id ... regression tests")
    /// for `thread_from_remote`'s `rest_id` handling: when the remote root
    /// comment carries a `rest_id` (GitHub's case — its GraphQL node `id`
    /// is not what the REST `in_reply_to` field needs), the namespaced
    /// root-comment ledger must be seeded from it so a later reply can find
    /// a REST-compatible ID; when `rest_id` is `None` (every other
    /// provider, or GitHub without a database id), the ledger must stay
    /// empty rather than being seeded with the wrong (GraphQL) id.
    #[test]
    fn should_seed_root_comment_ledger_from_rest_id_only_when_present() {
        let with_rest_id = thread_from_remote("github", &remote_thread_with_rest_id(Some("999")));
        assert_eq!(
            with_rest_id.root_comment_id("github"),
            Some("999"),
            "rest_id present: ledger must be seeded with the REST-compatible id, not the node id"
        );

        let without_rest_id = thread_from_remote("github", &remote_thread_with_rest_id(None));
        assert_eq!(
            without_rest_id.root_comment_id("github"),
            None,
            "rest_id absent: must not fabricate a ledger entry from the (REST-incompatible) node id"
        );
        // The bare mapping's own `id` field still carries the node id
        // (unaffected by rest_id), so display/lookup-by-native-id logic is
        // unchanged either way.
        assert!(without_rest_id.has_provider_id("github", "PRRT_1"));
    }

    fn gitlab_range_thread(is_outdated: bool, is_resolved: bool) -> RemoteReviewThread {
        RemoteReviewThread {
            id: "discussion-1".to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(12),
            side: RemoteCommentSide::Right,
            is_resolved,
            is_outdated,
            range: Some(crate::forge::remote_comments::RemoteReviewRange::new(10, 12).unwrap()),
            provider_native_anchor: Some(serde_json::json!({
                "position_type": "text",
                "head_sha": if is_outdated { "old-head" } else { "current-head" },
                "new_path": "src/lib.rs",
                "new_line": 12,
                "line_range": {
                    "start": {"type": "new", "new_line": 10},
                    "end": {"type": "new", "new_line": 12}
                }
            })),
            comments: vec![crate::forge::remote_comments::RemoteReviewComment {
                id: "100".to_string(),
                author: Some("alice".to_string()),
                body: "range review".to_string(),
                created_at: None,
                in_reply_to: None,
                url: String::new(),
                rest_id: None,
            }],
        }
    }

    #[test]
    fn should_import_gitlab_range_native_anchor_and_provider_stale_state() {
        let persisted = thread_from_remote("gitlab", &gitlab_range_thread(true, false));

        assert_eq!(persisted.thread.status(), ThreadStatus::Stale);
        assert!(persisted.thread.anchor().is_stale());
        assert!(matches!(
            persisted.thread.anchor().target(),
            crate::model::thread::AnchorTarget::Range {
                path,
                side: AnchorSide::New,
                start: 10,
                end: 12,
            } if path == "src/lib.rs"
        ));
        let mapping = persisted.provider_mapping("gitlab").unwrap();
        assert_eq!(
            mapping["range"],
            serde_json::json!({"start": 10, "end": 12})
        );
        assert_eq!(mapping["native_anchor"]["head_sha"], "old-head");
        assert_eq!(mapping["is_outdated"], true);
    }

    #[test]
    fn should_monotonically_apply_provider_stale_on_repeat_import_and_reopen() {
        let mut existing = thread_from_remote("gitlab", &gitlab_range_thread(false, false));
        existing.thread.resolve();
        let fresh = thread_from_remote("gitlab", &gitlab_range_thread(true, false));

        merge_remote_thread_into_existing(&mut existing, fresh, "gitlab");

        assert_eq!(existing.thread.status(), ThreadStatus::Resolved);
        assert!(existing.thread.anchor().is_stale());
        assert_eq!(
            existing.provider_mapping("gitlab").unwrap()["is_outdated"],
            true
        );
        assert!(existing.thread.reopen());
        assert_eq!(existing.thread.status(), ThreadStatus::Stale);
    }

    #[test]
    fn should_not_replace_local_ambiguous_state_on_outdated_repeat_import() {
        let mut existing = thread_from_remote("gitlab", &gitlab_range_thread(false, false));
        let mut thread_json = serde_json::to_value(&existing.thread).unwrap();
        thread_json["status"] = serde_json::json!("ambiguous");
        thread_json["anchor"]["state"] = serde_json::json!("ambiguous");
        existing.thread = serde_json::from_value(thread_json).unwrap();
        let fresh = thread_from_remote("gitlab", &gitlab_range_thread(true, false));

        merge_remote_thread_into_existing(&mut existing, fresh, "gitlab");

        assert_eq!(existing.thread.status(), ThreadStatus::Ambiguous);
        assert!(existing.thread.anchor().is_ambiguous());
        assert_eq!(
            existing.provider_mapping("gitlab").unwrap()["is_outdated"],
            true
        );
    }

    /// Regression test (audit finding: "cross-provider mapping
    /// serialization regression tests") pinning that each provider's
    /// distinctively-shaped `provider_mappings` JSON payload (GitHub's
    /// `root_comment_id`, GitLab's discussion `id`, Azure's nested
    /// `threadContext`, Gitea/Forgejo's plain numeric-string `id`) survives
    /// a full `PersistedThread` JSON round-trip byte-for-byte, and that no
    /// provider's payload bleeds into another's key — guarding against a
    /// merge regression silently dropping or cross-contaminating one
    /// provider's mapping while wiring up a sibling provider.
    #[test]
    fn should_round_trip_distinct_provider_mapping_shapes_without_cross_contamination() {
        let comment = legacy_comment(DEFAULT_AUTHOR, "note");
        let mut persisted = thread_from_legacy_comment(Anchor::review(), &comment);

        persisted.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "root_comment_id": "999"}),
        );
        persisted.upsert_provider_mapping("gitlab", serde_json::json!({"id": "note-42"}));
        persisted.upsert_provider_mapping(
            "azure-devops",
            serde_json::json!({
                "id": "501",
                "is_resolved": false,
                "threadContext": {"filePath": "/src/lib.rs", "rightFileStart": {"line": 10}},
            }),
        );
        persisted.upsert_provider_mapping("gitea", serde_json::json!({"id": "77"}));
        persisted.upsert_provider_mapping("forgejo", serde_json::json!({"id": "78"}));

        let json = serde_json::to_value(&persisted).unwrap();
        let restored: PersistedThread = serde_json::from_value(json).unwrap();

        assert_eq!(restored, persisted, "full round-trip must be lossless");
        assert_eq!(restored.provider_mappings.len(), 5);
        assert_eq!(
            restored.provider_mapping("github").unwrap()["root_comment_id"],
            serde_json::json!("999")
        );
        assert_eq!(
            restored.provider_mapping("gitlab").unwrap()["id"],
            serde_json::json!("note-42")
        );
        assert_eq!(
            restored.provider_mapping("azure-devops").unwrap()["threadContext"]["filePath"],
            serde_json::json!("/src/lib.rs")
        );
        assert_eq!(
            restored.provider_mapping("gitea").unwrap()["id"],
            serde_json::json!("77")
        );
        assert_eq!(
            restored.provider_mapping("forgejo").unwrap()["id"],
            serde_json::json!("78")
        );
        // Cross-contamination guard: gitlab's payload never picked up
        // azure's `threadContext` key or github's `root_comment_id`, etc.
        assert!(
            restored
                .provider_mapping("gitlab")
                .unwrap()
                .get("threadContext")
                .is_none()
        );
        assert!(
            restored
                .provider_mapping("gitea")
                .unwrap()
                .get("root_comment_id")
                .is_none()
        );
        // No credential-shaped keys ever appear in a provider mapping —
        // only opaque forge-native ids/anchors, never tokens/secrets.
        for provider in ["github", "gitlab", "azure-devops", "gitea", "forgejo"] {
            let mapping = restored.provider_mapping(provider).unwrap().to_string();
            let lower = mapping.to_ascii_lowercase();
            assert!(
                !lower.contains("token")
                    && !lower.contains("secret")
                    && !lower.contains("password"),
                "provider={provider} mapping must never carry credential-shaped fields: {mapping}"
            );
        }
    }
}
