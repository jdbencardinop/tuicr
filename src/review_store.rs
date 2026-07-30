use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::error::{Result, TuicrError};
use crate::model::thread::{
    Anchor, ProviderRemap, ThreadAnchorRefresh, ThreadAuthor, ThreadComment, ThreadId,
};
use crate::model::{Comment, CommentType, LineRange, LineSide, PersistedThread, ReviewSession};
use crate::persistence::manifest::{ManifestEntry, ManifestKind};
use crate::persistence::storage;

/// File-backed access to persisted tuicr review sessions.
#[derive(Debug, Clone, Default)]
pub struct ReviewStore {
    reviews_dir: Option<PathBuf>,
}

impl ReviewStore {
    /// Use tuicr's platform data directory.
    pub fn new() -> Self {
        Self::default()
    }

    /// Use an explicit reviews directory. This is primarily useful for
    /// wrappers, tests, and tools that want isolated session storage.
    pub fn with_reviews_dir(reviews_dir: impl Into<PathBuf>) -> Self {
        Self {
            reviews_dir: Some(reviews_dir.into()),
        }
    }

    /// List persisted sessions for a repo selector — a checkout path or a
    /// forge coordinate like `owner/repo`. A checkout path matches its own
    /// local sessions and, via its `origin` remote, any PR sessions for the
    /// same repo; a coordinate matches local and PR sessions by `owner/repo`.
    pub fn list_sessions_for_repo(
        &self,
        selector: impl AsRef<Path>,
    ) -> Result<Vec<SessionSummary>> {
        let reviews_dir = self.reviews_dir()?;
        let entries = storage::list_sessions_for_selector_in_dir(&reviews_dir, selector.as_ref())?;
        let active_paths = storage::active_session_paths_in_dir(&reviews_dir)?;
        Ok(entries
            .into_iter()
            .map(|(slug, entry)| summary_from_entry(&reviews_dir, &active_paths, slug, entry))
            .collect())
    }

    /// List every persisted session, local and PR, newest first. Backs
    /// `tuicr review list --all` for when the caller does not know the repo.
    pub fn list_all_sessions(&self) -> Result<Vec<SessionSummary>> {
        let reviews_dir = self.reviews_dir()?;
        let entries = storage::list_all_sessions_in_dir(&reviews_dir)?;
        let active_paths = storage::active_session_paths_in_dir(&reviews_dir)?;
        Ok(entries
            .into_iter()
            .map(|(slug, entry)| summary_from_entry(&reviews_dir, &active_paths, slug, entry))
            .collect())
    }

    /// Resolve a PR session to its [`SessionRef`] from a PR slug
    /// (`gh:owner/repo/pr/<n>`). Returns `None` when no PR session is
    /// persisted for that slug.
    pub fn resolve_pr_session(&self, slug: &str) -> Result<Option<SessionRef>> {
        let reviews_dir = self.reviews_dir()?;
        Ok(storage::pr_session_path_in_dir(&reviews_dir, slug)?.map(SessionRef::from_path))
    }

    /// Look up the durable review session for a PR's *lineage* (forge kind,
    /// host, owner/repo, PR number), regardless of the head SHA in `key`.
    /// This is the additive lookup requirement 6 asks for: it lets a
    /// caller reuse an existing review's threads/replies/provider mappings
    /// across a PR head advance instead of discarding them, without
    /// changing the exact head-matching semantics [`Self::get_review`]/
    /// [`crate::persistence::load_pr_session`] rely on today.
    ///
    /// Returns `None` if no session has ever been saved for this PR's
    /// lineage. The returned session's `pr_session_key.head_sha` reflects
    /// whatever head it was last saved under; callers that want to rebind
    /// it to a new head should refresh anchors (see
    /// [`Self::refresh_thread_anchors`]) and save under the new key.
    pub fn find_pr_session_by_lineage(
        &self,
        key: &crate::forge::traits::PrSessionKey,
    ) -> Result<Option<(SessionRef, ReviewSession)>> {
        let reviews_dir = self.reviews_dir()?;
        Ok(storage::load_pr_session_lineage_for_dir(&reviews_dir, key)?
            .map(|(path, session)| (SessionRef::from_path(path), session)))
    }

    /// Rebind an existing durable review to a new PR head, completing the
    /// lineage lookup [`Self::find_pr_session_by_lineage`] performs into an
    /// actual migration: find the review by lineage (forge/host/repo/PR
    /// number) regardless of which head it was last saved under, update
    /// its `pr_session_key.head_sha` to `new_key.head_sha`, and save it
    /// under the new head's canonical path. Threads, replies, statuses and
    /// provider mappings are carried forward unchanged; this does not
    /// itself refresh anchors against the new head's diff content (call
    /// [`Self::refresh_thread_anchors`] with the new content for that).
    ///
    /// Idempotent: calling this again with the same `new_key` after it has
    /// already been rebound finds the review (now already at `new_key`'s
    /// head) and re-saves it unchanged. Returns `Ok(None)` if no review
    /// exists yet for this lineage; callers should fall back to creating a
    /// fresh session in that case, exactly as they do today.
    ///
    /// The lineage lookup, the `pr_session_key` mutation, and the save all
    /// happen inside one `.tuicr.lock`-protected critical section (see
    /// [`storage::rebind_pr_session_to_head_in_dir`]) — not a lookup
    /// followed by a separately-locked save — so a concurrent writer (e.g.
    /// [`Self::reply_to_thread`] appending to the same old-head session)
    /// can never have its update silently discarded by this function
    /// saving a stale pre-lock snapshot over it. See that function's doc
    /// comment for the one residual limitation this does *not* close: a
    /// caller holding a stale, pre-rebind [`SessionRef`] for the old head
    /// can still durably write to that now-orphaned file afterwards; it
    /// just won't be found by lineage going forward.
    pub fn rebind_pr_session_to_head(
        &self,
        new_key: &crate::forge::traits::PrSessionKey,
    ) -> Result<Option<(SessionRef, ReviewSession)>> {
        let reviews_dir = self.reviews_dir()?;
        let Some((path, session)) =
            storage::rebind_pr_session_to_head_in_dir(&reviews_dir, new_key)?
        else {
            return Ok(None);
        };
        Ok(Some((SessionRef::from_path(path), session)))
    }

    /// Load a persisted review session.
    pub fn get_review(&self, session_ref: &SessionRef) -> Result<ReviewSession> {
        storage::load_session(session_ref.path())
    }

    /// Add a local draft comment to a persisted session and save it.
    pub fn add_comment(
        &self,
        session_ref: &SessionRef,
        request: AddCommentRequest,
    ) -> Result<Comment> {
        let reviews_dir = self.reviews_dir()?;
        let (_session, comment) =
            storage::update_session_in_dir(session_ref.path(), &reviews_dir, |session| {
                add_comment_to_session(session, request)
            })?;
        Ok(comment)
    }

    /// List every durable thread in a persisted session, migrating legacy
    /// `Comment`s into threads first if the session predates
    /// `CURRENT_SESSION_VERSION` (see
    /// [`crate::model::ReviewSession::migrate_legacy_comments_to_threads`]).
    /// Read-only: does not save, so a v1.3 session's on-disk file is
    /// unchanged by merely listing its threads.
    pub fn list_threads(&self, session_ref: &SessionRef) -> Result<Vec<PersistedThread>> {
        let session = self.get_review(session_ref)?;
        Ok(session.threads().to_vec())
    }

    /// Fetch a single thread by ID.
    pub fn get_thread(
        &self,
        session_ref: &SessionRef,
        thread_id: &ThreadId,
    ) -> Result<Option<PersistedThread>> {
        let session = self.get_review(session_ref)?;
        Ok(session.find_thread(thread_id).cloned())
    }

    /// Open a new durable thread and save it. `target` reuses the same
    /// [`CommentTarget`] vocabulary as [`Self::add_comment`] so CLI/library
    /// callers share one target-parsing path for both legacy comments and
    /// threads.
    pub fn add_thread(
        &self,
        session_ref: &SessionRef,
        request: AddThreadRequest,
    ) -> Result<PersistedThread> {
        let reviews_dir = self.reviews_dir()?;
        let (session, thread_id) =
            storage::update_session_in_dir(session_ref.path(), &reviews_dir, |session| {
                add_thread_to_session(session, request)
            })?;
        Ok(session
            .find_thread(&thread_id)
            .cloned()
            .expect("thread just added must be present"))
    }

    /// Append a reply to an existing thread and save it.
    pub fn reply_to_thread(
        &self,
        session_ref: &SessionRef,
        thread_id: &ThreadId,
        author: ThreadAuthor,
        body: impl Into<String>,
    ) -> Result<PersistedThread> {
        let reviews_dir = self.reviews_dir()?;
        let body = body.into();
        let (session, ()) =
            storage::update_session_in_dir(session_ref.path(), &reviews_dir, |session| {
                let thread = session.find_thread_mut(thread_id).ok_or_else(|| {
                    TuicrError::InvalidInput(format!("thread '{}' not found", thread_id.as_str()))
                })?;
                thread.thread.reply(ThreadComment::new(author, body));
                session.updated_at = Utc::now();
                Ok(())
            })?;
        Ok(session
            .find_thread(thread_id)
            .cloned()
            .expect("thread just replied to must be present"))
    }

    /// Resolve a thread. Returns `false` (no-op) if the thread was already
    /// `Dismissed`, per [`crate::model::Thread::resolve`].
    pub fn resolve_thread(&self, session_ref: &SessionRef, thread_id: &ThreadId) -> Result<bool> {
        self.mutate_thread_status(session_ref, thread_id, |thread| thread.resolve())
    }

    /// Reopen a resolved thread. Returns `false` (no-op) if the thread was
    /// not `Resolved` (including if it was `Dismissed`, which is terminal),
    /// per [`crate::model::Thread::reopen`].
    pub fn reopen_thread(&self, session_ref: &SessionRef, thread_id: &ThreadId) -> Result<bool> {
        self.mutate_thread_status(session_ref, thread_id, |thread| thread.reopen())
    }

    /// Dismiss a thread ("won't fix"). Terminal: a dismissed thread can
    /// never be resolved or reopened again.
    pub fn dismiss_thread(&self, session_ref: &SessionRef, thread_id: &ThreadId) -> Result<()> {
        self.mutate_thread_status(session_ref, thread_id, |thread| {
            thread.dismiss();
            true
        })?;
        Ok(())
    }

    fn mutate_thread_status(
        &self,
        session_ref: &SessionRef,
        thread_id: &ThreadId,
        mutate: impl FnOnce(&mut crate::model::Thread) -> bool,
    ) -> Result<bool> {
        let reviews_dir = self.reviews_dir()?;
        let (_session, changed) =
            storage::update_session_in_dir(session_ref.path(), &reviews_dir, |session| {
                let thread = session.find_thread_mut(thread_id).ok_or_else(|| {
                    TuicrError::InvalidInput(format!("thread '{}' not found", thread_id.as_str()))
                })?;
                let changed = mutate(&mut thread.thread);
                session.updated_at = Utc::now();
                Ok(changed)
            })?;
        Ok(changed)
    }

    /// Safely re-evaluate every thread's anchor against updated file
    /// content and save the result. `new_lines` supplies the full 0-based
    /// line slice for each path that changed; paths absent from the map are
    /// left untouched. `provider_remaps` lets a (future) provider adapter
    /// supply an exact remap for specific threads, which always wins over
    /// unique-context relocation for that thread — see
    /// [`crate::model::Thread::refresh_anchor_with_remap`]. Closed threads
    /// (`Resolved`/`Dismissed`) are frozen and never touched, per the same
    /// contract.
    pub fn refresh_thread_anchors(
        &self,
        session_ref: &SessionRef,
        new_lines: &HashMap<PathBuf, Vec<String>>,
        provider_remaps: &HashMap<ThreadId, ProviderRemap>,
    ) -> Result<Vec<(ThreadId, ThreadAnchorRefresh)>> {
        let reviews_dir = self.reviews_dir()?;
        let (_session, results) =
            storage::update_session_in_dir(session_ref.path(), &reviews_dir, |session| {
                let mut results = Vec::new();
                for thread in session.threads.iter_mut() {
                    let Some(path) = thread.thread.anchor().target().path() else {
                        continue;
                    };
                    let Some(lines) = new_lines.get(Path::new(path)) else {
                        continue;
                    };
                    let lines_ref: Vec<&str> = lines.iter().map(String::as_str).collect();
                    let remap = provider_remaps.get(thread.id());
                    let refresh = thread.refresh_anchor_with_remap(&lines_ref, remap)?;
                    results.push((thread.id().clone(), refresh));
                }
                session.updated_at = Utc::now();
                Ok(results)
            })?;
        Ok(results)
    }

    /// Insert or overwrite a thread's provider-native mapping and save.
    /// Idempotent: calling this repeatedly with the same `provider` key
    /// replaces the stored payload rather than accumulating duplicates.
    pub fn upsert_thread_provider_mapping(
        &self,
        session_ref: &SessionRef,
        thread_id: &ThreadId,
        provider: &str,
        mapping: serde_json::Value,
    ) -> Result<PersistedThread> {
        let reviews_dir = self.reviews_dir()?;
        let (session, ()) =
            storage::update_session_in_dir(session_ref.path(), &reviews_dir, |session| {
                let thread = session.find_thread_mut(thread_id).ok_or_else(|| {
                    TuicrError::InvalidInput(format!("thread '{}' not found", thread_id.as_str()))
                })?;
                thread.upsert_provider_mapping(provider, mapping);
                session.updated_at = Utc::now();
                Ok(())
            })?;
        Ok(session
            .find_thread(thread_id)
            .cloned()
            .expect("thread just mapped must be present"))
    }

    /// Save a session through this store's storage root.
    pub fn save_review(&self, session: &ReviewSession) -> Result<SessionRef> {
        let reviews_dir = self.reviews_dir()?;
        storage::save_session_in_dir(session, &reviews_dir).map(SessionRef::from_path)
    }

    fn reviews_dir(&self) -> Result<PathBuf> {
        match &self.reviews_dir {
            Some(path) => Ok(path.clone()),
            None => storage::get_reviews_dir(),
        }
    }
}

/// Build a [`SessionSummary`] from a manifest entry, resolving its absolute
/// path and active state. Shared by the per-repo and `--all` listings.
fn summary_from_entry(
    reviews_dir: &Path,
    active_paths: &std::collections::HashSet<PathBuf>,
    slug: String,
    entry: ManifestEntry,
) -> SessionSummary {
    let path = reviews_dir.join(entry.path);
    let active = active_paths.contains(&storage::normalize_path_for_comparison(&path));
    let kind = match entry.kind {
        ManifestKind::Local => SessionKind::Local,
        ManifestKind::Pr { .. } => SessionKind::Pr,
    };
    SessionSummary {
        session_ref: SessionRef::from_path(path),
        slug,
        kind,
        updated_at: entry.updated_at,
        comment_count: entry.display.comment_count,
        reviewed_count: entry.display.reviewed_count,
        file_count: entry.display.file_count,
        anchor: entry.display.anchor,
        active,
    }
}

/// Opaque reference to a persisted review session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionRef {
    path: PathBuf,
}

impl SessionRef {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Whether a persisted session tracks a local checkout or a forge PR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Local,
    Pr,
}

impl SessionKind {
    pub fn id(self) -> &'static str {
        match self {
            SessionKind::Local => "local",
            SessionKind::Pr => "pr",
        }
    }
}

/// Lightweight metadata for a persisted session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub session_ref: SessionRef,
    pub slug: String,
    pub kind: SessionKind,
    pub updated_at: DateTime<Utc>,
    pub comment_count: usize,
    pub reviewed_count: usize,
    pub file_count: usize,
    pub anchor: String,
    pub active: bool,
}

/// Request to add a local draft comment to a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddCommentRequest {
    pub target: CommentTarget,
    pub content: String,
    pub comment_type: CommentType,
    /// Author to stamp on the resulting comment. Caller is responsible for
    /// picking a sensible default (`Comment::DEFAULT_AUTHOR`) when none is
    /// supplied.
    pub author: String,
    /// Commit SHA to stamp on the comment when it was created while the
    /// inline commit selector showed exactly one commit. `None` for
    /// review-level comments and full-range selections. Library callers
    /// (the `review add` CLI) leave this `None`.
    pub commit_id: Option<String>,
}

/// Where a new local draft comment should be attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommentTarget {
    Review,
    File {
        path: PathBuf,
    },
    Line {
        path: PathBuf,
        line: u32,
        side: LineSide,
    },
    LineRange {
        path: PathBuf,
        range: LineRange,
        side: LineSide,
    },
}

/// Add a local draft comment to an in-memory session.
///
/// This is the shared primitive used by the TUI and by [`ReviewStore`].
pub fn add_comment_to_session(
    session: &mut ReviewSession,
    request: AddCommentRequest,
) -> Result<Comment> {
    let content = request.content.trim().to_string();
    if content.is_empty() {
        return Err(TuicrError::InvalidInput(
            "comment cannot be empty".to_string(),
        ));
    }

    let author = request.author;
    let commit_id = request.commit_id;
    let comment = match request.target {
        CommentTarget::Review => {
            let comment = Comment::new(content, request.comment_type, None).with_author(author);
            session.review_comments.push(comment.clone());
            comment
        }
        CommentTarget::File { path } => {
            let review = file_review_mut(session, &path)?;
            let mut comment = Comment::new(content, request.comment_type, None).with_author(author);
            if let Some(sha) = &commit_id {
                comment = comment.with_commit_id(sha.clone());
            }
            review.add_file_comment(comment.clone());
            comment
        }
        CommentTarget::Line { path, line, side } => {
            let review = file_review_mut(session, &path)?;
            let mut comment =
                Comment::new(content, request.comment_type, Some(side)).with_author(author);
            if let Some(sha) = &commit_id {
                comment = comment.with_commit_id(sha.clone());
            }
            review.add_line_comment(line, comment.clone());
            comment
        }
        CommentTarget::LineRange { path, range, side } => {
            let review = file_review_mut(session, &path)?;
            let mut comment =
                Comment::new_with_range(content, request.comment_type, Some(side), range)
                    .with_author(author);
            if let Some(sha) = &commit_id {
                comment = comment.with_commit_id(sha.clone());
            }
            review.add_line_comment(range.end, comment.clone());
            comment
        }
    };

    session.updated_at = Utc::now();
    Ok(comment)
}

fn file_review_mut<'a>(
    session: &'a mut ReviewSession,
    path: &Path,
) -> Result<&'a mut crate::model::review::FileReview> {
    session.get_file_mut(&path.to_path_buf()).ok_or_else(|| {
        TuicrError::InvalidInput(format!("session does not contain file {}", path.display()))
    })
}

/// Request to open a new durable thread on a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddThreadRequest {
    /// Reuses [`CommentTarget`] so thread and legacy-comment targets share
    /// one CLI/library parsing path.
    pub target: CommentTarget,
    pub body: String,
    pub author: ThreadAuthor,
}

/// Convert a [`CommentTarget`] into the [`Anchor`] vocabulary used by
/// durable threads. `Line`/`Range` sides map 1:1 (legacy comments never
/// express `Both`).
fn anchor_for_target(target: &CommentTarget) -> Anchor {
    match target {
        CommentTarget::Review => Anchor::review(),
        CommentTarget::File { path } => Anchor::file(path.to_string_lossy().to_string()),
        CommentTarget::Line { path, line, side } => Anchor::line(
            path.to_string_lossy().to_string(),
            anchor_side(*side),
            *line,
        ),
        CommentTarget::LineRange { path, range, side } => Anchor::range(
            path.to_string_lossy().to_string(),
            anchor_side(*side),
            range.start,
            range.end,
        )
        .expect("LineRange is always normalized start <= end"),
    }
}

fn anchor_side(side: LineSide) -> crate::model::thread::AnchorSide {
    match side {
        LineSide::Old => crate::model::thread::AnchorSide::Old,
        LineSide::New => crate::model::thread::AnchorSide::New,
    }
}

/// Open a new durable thread in an in-memory session, validating (for
/// `File`/`Line`/`LineRange` targets) that the target file is registered in
/// the session, exactly like [`add_comment_to_session`].
///
/// This is the shared primitive used by [`ReviewStore::add_thread`]; kept
/// as a free function so future TUI wiring can call it directly on an
/// in-memory session, matching the existing `add_comment_to_session`
/// convention.
pub fn add_thread_to_session(
    session: &mut ReviewSession,
    request: AddThreadRequest,
) -> Result<ThreadId> {
    let body = request.body.trim().to_string();
    if body.is_empty() {
        return Err(TuicrError::InvalidInput(
            "thread comment cannot be empty".to_string(),
        ));
    }
    if let CommentTarget::File { path }
    | CommentTarget::Line { path, .. }
    | CommentTarget::LineRange { path, .. } = &request.target
    {
        file_review_mut(session, path)?;
    }

    let anchor = anchor_for_target(&request.target);
    let root = ThreadComment::new(request.author, body);
    Ok(session.add_thread(anchor, root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CURRENT_SESSION_VERSION, FileStatus, SessionDiffSource};

    fn test_session(repo_path: PathBuf) -> ReviewSession {
        let mut session = ReviewSession::new(
            repo_path,
            "abc1234".to_string(),
            Some("main".to_string()),
            SessionDiffSource::WorkingTree,
        );
        session.add_file(PathBuf::from("src/main.rs"), FileStatus::Modified, 0);
        session
    }

    #[test]
    fn should_add_review_level_comment_to_session() {
        let mut session = test_session(PathBuf::from("/repo"));

        let comment = add_comment_to_session(
            &mut session,
            AddCommentRequest {
                target: CommentTarget::Review,
                content: "looks good".to_string(),
                comment_type: CommentType::from_id("praise"),
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                commit_id: None,
            },
        )
        .unwrap();

        assert_eq!(session.review_comments, vec![comment]);
    }

    #[test]
    fn should_add_file_comment_to_session() {
        let mut session = test_session(PathBuf::from("/repo"));

        let comment = add_comment_to_session(
            &mut session,
            AddCommentRequest {
                target: CommentTarget::File {
                    path: PathBuf::from("src/main.rs"),
                },
                content: "file note".to_string(),
                comment_type: CommentType::from_id("note"),
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                commit_id: None,
            },
        )
        .unwrap();

        let review = session.files.get(&PathBuf::from("src/main.rs")).unwrap();
        assert_eq!(review.file_comments, vec![comment]);
    }

    #[test]
    fn should_add_line_range_comment_by_range_end() {
        let mut session = test_session(PathBuf::from("/repo"));
        let range = LineRange::new(10, 12);

        let comment = add_comment_to_session(
            &mut session,
            AddCommentRequest {
                target: CommentTarget::LineRange {
                    path: PathBuf::from("src/main.rs"),
                    range,
                    side: LineSide::New,
                },
                content: "range note".to_string(),
                comment_type: CommentType::from_id("suggestion"),
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                commit_id: None,
            },
        )
        .unwrap();

        let review = session.files.get(&PathBuf::from("src/main.rs")).unwrap();
        assert_eq!(review.line_comments.get(&12), Some(&vec![comment]));
    }

    #[test]
    fn should_reject_unknown_file() {
        let mut session = test_session(PathBuf::from("/repo"));

        let err = add_comment_to_session(
            &mut session,
            AddCommentRequest {
                target: CommentTarget::File {
                    path: PathBuf::from("missing.rs"),
                },
                content: "note".to_string(),
                comment_type: CommentType::from_id("note"),
                author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                commit_id: None,
            },
        )
        .unwrap_err();

        assert!(matches!(err, TuicrError::InvalidInput(_)));
    }

    #[test]
    fn should_list_and_update_sessions_through_store() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(reviews_dir.clone());
        let session = test_session(repo.clone());
        let session_ref = store.save_review(&session).unwrap();

        let listed = store.list_sessions_for_repo(&repo).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_ref, session_ref);
        assert_eq!(listed[0].file_count, 1);
        assert_eq!(listed[0].comment_count, 0);
        assert!(!listed[0].active);

        crate::persistence::storage::mark_session_active_in_dir(
            &session,
            session_ref.path(),
            &reviews_dir,
        )
        .unwrap();
        let listed = store.list_sessions_for_repo(&repo).unwrap();
        assert!(listed[0].active);

        store
            .add_comment(
                &session_ref,
                AddCommentRequest {
                    target: CommentTarget::Line {
                        path: PathBuf::from("src/main.rs"),
                        line: 7,
                        side: LineSide::New,
                    },
                    content: "line note".to_string(),
                    comment_type: CommentType::from_id("note"),
                    author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                    commit_id: None,
                },
            )
            .unwrap();

        let loaded = store.get_review(&session_ref).unwrap();
        let review = loaded.files.get(&PathBuf::from("src/main.rs")).unwrap();
        assert_eq!(review.line_comments.get(&7).unwrap().len(), 1);

        let listed = store.list_sessions_for_repo(&repo).unwrap();
        assert_eq!(listed[0].comment_count, 1);
    }

    #[test]
    fn should_rebind_pr_session_to_new_head_and_carry_threads_forward() {
        use crate::forge::traits::{ForgeRepository, PrSessionKey};

        let temp = tempfile::tempdir().unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(reviews_dir);

        let repository = ForgeRepository::github("github.com", "acme", "widgets");
        let old_key = PrSessionKey::new(repository.clone(), 42, "old-head-sha");
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/acme/widgets"),
            old_key.head_sha.clone(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(old_key.clone());
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, 0);
        let session_ref = store.save_review(&session).unwrap();

        let thread = store
            .add_thread(
                &session_ref,
                AddThreadRequest {
                    target: CommentTarget::Line {
                        path: PathBuf::from("src/lib.rs"),
                        line: 1,
                        side: LineSide::New,
                    },
                    body: "please fix".to_string(),
                    author: ThreadAuthor::human("reviewer"),
                },
            )
            .unwrap();

        // Rebind to a new head: the review is found by lineage (repo + PR
        // number) regardless of the old head, its `head_sha` is updated,
        // and the thread created above survives the rebind untouched.
        let new_key = PrSessionKey::new(repository, 42, "new-head-sha");
        let (new_ref, rebound) = store
            .rebind_pr_session_to_head(&new_key)
            .unwrap()
            .expect("lineage hit for rebind");
        assert_eq!(
            rebound.pr_session_key.as_ref().unwrap().head_sha,
            "new-head-sha"
        );
        assert_eq!(rebound.threads().len(), 1);
        assert_eq!(rebound.threads()[0].thread.id(), thread.thread.id());

        // The exact old-head lookup no longer resolves (it was rebound),
        // but the new head resolves directly, and the lineage lookup finds
        // it under the new head too.
        assert!(store.get_review(&session_ref).is_ok());
        let reloaded = store.get_review(&new_ref).unwrap();
        assert_eq!(reloaded.threads().len(), 1);

        let (_, lineage_session) = store
            .find_pr_session_by_lineage(&new_key)
            .unwrap()
            .expect("lineage lookup after rebind");
        assert_eq!(lineage_session.pr_session_key.as_ref().unwrap(), &new_key);
    }

    #[test]
    fn should_rebind_idempotently_when_called_twice_for_same_head() {
        use crate::forge::traits::{ForgeRepository, PrSessionKey};

        let temp = tempfile::tempdir().unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(reviews_dir);

        let repository = ForgeRepository::github("github.com", "acme", "widgets");
        let old_key = PrSessionKey::new(repository.clone(), 7, "head-a");
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/acme/widgets"),
            old_key.head_sha.clone(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(old_key);
        store.save_review(&session).unwrap();

        let new_key = PrSessionKey::new(repository, 7, "head-b");
        let first = store
            .rebind_pr_session_to_head(&new_key)
            .unwrap()
            .expect("first rebind hit");
        let second = store
            .rebind_pr_session_to_head(&new_key)
            .unwrap()
            .expect("second rebind hit");

        assert_eq!(first.0.path(), second.0.path());
        assert_eq!(first.1.id, second.1.id);
        assert_eq!(second.1.pr_session_key.as_ref().unwrap().head_sha, "head-b");
    }

    #[test]
    fn should_return_none_rebinding_a_pr_with_no_prior_session() {
        use crate::forge::traits::{ForgeRepository, PrSessionKey};

        let temp = tempfile::tempdir().unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(reviews_dir);

        let repository = ForgeRepository::github("github.com", "acme", "widgets");
        let key = PrSessionKey::new(repository, 999, "some-head");
        assert!(store.rebind_pr_session_to_head(&key).unwrap().is_none());
    }

    #[test]
    fn should_read_lineage_inside_the_lock_when_rebinding_and_not_miss_a_concurrent_write() {
        // Deterministically proves `rebind_pr_session_to_head`'s lineage
        // read now happens *inside* the shared `.tuicr.lock` critical
        // section rather than before it, closing the lost-update race the
        // previous unlocked-read-then-locked-write implementation had.
        //
        // A real multi-thread race between a writer and a rebind isn't a
        // reliable way to prove this: whichever operation wins the lock
        // first is a *valid* outcome even under the fix (the fix only
        // guarantees ordering relative to the lock, not a specific
        // winner), so a naive race would be flaky either way. Instead this
        // test controls the interleaving explicitly: it holds the
        // `.tuicr.lock` file itself (using the test process's own live
        // PID, so the storage module's staleness check treats it as held
        // by a running process and the rebind call underneath is forced
        // to genuinely block/retry rather than proceed), performs a
        // direct on-disk write simulating a writer's fully completed,
        // already-unlocked update while that fake lock is held, then
        // releases it. If the lineage read happens inside the lock (the
        // fix), the now-unblocked rebind is guaranteed to observe the
        // simulated write, because it cannot acquire the lock -- and thus
        // cannot read -- until after the write has landed. Under the old
        // code (whose unlocked read fires immediately on the background
        // thread, without ever waiting on this fake lock at all) this
        // assertion fails, since the stale pre-write snapshot is what gets
        // captured and later saved back over the rebound session.
        use crate::forge::traits::{ForgeRepository, PrSessionKey};
        use std::sync::Arc;

        let temp = tempfile::tempdir().unwrap();
        let reviews_dir = temp.path().join("reviews");
        let store = ReviewStore::with_reviews_dir(reviews_dir.clone());

        let repository = ForgeRepository::github("github.com", "acme", "widgets");
        let old_key = PrSessionKey::new(repository.clone(), 55, "old-head-sha");
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/acme/widgets"),
            old_key.head_sha.clone(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.pr_session_key = Some(old_key.clone());
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, 0);
        let session_ref = store.save_review(&session).unwrap();

        // Simulate the review storage lock already being held by a live
        // process: writing the test's own PID means `process_is_running`
        // (used by the staleness check) reports it as alive, so any
        // `with_reviews_dir_lock` caller genuinely blocks/retries instead
        // of treating it as stale and barging in.
        std::fs::create_dir_all(&reviews_dir).unwrap();
        let lock_path = reviews_dir.join(".tuicr.lock");
        std::fs::write(&lock_path, format!("{}\n", std::process::id())).unwrap();

        let new_key = PrSessionKey::new(repository, 55, "new-head-sha");
        let store = Arc::new(store);
        let store_bg = Arc::clone(&store);
        let handle = std::thread::spawn(move || store_bg.rebind_pr_session_to_head(&new_key));

        // Give the background thread a chance to reach (and start
        // blocking on) the lock's acquire-retry loop before we simulate
        // the concurrent write; not required for correctness (see comment
        // above -- the write always happens before we remove the lock
        // either way), but it exercises genuine blocking rather than a
        // trivially-uncontended lock.
        std::thread::sleep(std::time::Duration::from_millis(150));

        // Simulate a concurrent writer's fully completed update (e.g. an
        // `update_session_in_dir`-based reply) landing on the old-head
        // session file while the lock is "held".
        let mut concurrently_written = store.get_review(&session_ref).unwrap();
        concurrently_written.add_file(PathBuf::from("src/extra.rs"), FileStatus::Added, 0);
        std::fs::write(
            session_ref.path(),
            serde_json::to_string_pretty(&concurrently_written).unwrap(),
        )
        .unwrap();

        // Release the fake lock; the blocked rebind can now proceed.
        std::fs::remove_file(&lock_path).unwrap();

        let (_, rebound) = handle
            .join()
            .unwrap()
            .unwrap()
            .expect("lineage hit for rebind");
        assert!(
            rebound.files.contains_key(&PathBuf::from("src/extra.rs")),
            "rebind must observe the concurrently-completed write made while it was \
             blocked on the lock, not a stale pre-lock snapshot"
        );
    }

    mod thread_store_tests {
        use super::*;
        use crate::model::thread::{
            Anchor, AnchorRelocation, AnchorState, ProviderRemap, ThreadAnchorRefresh,
            ThreadAuthor, ThreadStatus,
        };
        use std::collections::HashMap;

        fn store_with_session() -> (tempfile::TempDir, ReviewStore, SessionRef) {
            let temp = tempfile::tempdir().unwrap();
            let repo = temp.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let reviews_dir = temp.path().join("reviews");
            let store = ReviewStore::with_reviews_dir(reviews_dir);
            let session = test_session(repo);
            let session_ref = store.save_review(&session).unwrap();
            (temp, store, session_ref)
        }

        #[test]
        fn should_add_thread_and_persist_it() {
            let (_temp, store, session_ref) = store_with_session();

            let thread = store
                .add_thread(
                    &session_ref,
                    AddThreadRequest {
                        target: CommentTarget::Review,
                        body: "please double-check this".to_string(),
                        author: ThreadAuthor::human("alice"),
                    },
                )
                .unwrap();

            assert_eq!(thread.thread.status(), ThreadStatus::Open);
            let reloaded = store.get_review(&session_ref).unwrap();
            assert_eq!(reloaded.threads().len(), 1);
            assert_eq!(
                reloaded.threads()[0].thread.root().unwrap().body,
                "please double-check this"
            );
        }

        #[test]
        fn should_serialize_concurrent_thread_appends_with_no_lost_updates() {
            // Reuses the existing `.tuicr.lock` directory-level lock (via
            // `update_session_in_dir`/`with_reviews_dir_lock`) rather than
            // any new locking primitive: N real OS threads race to append
            // a reply to the same thread through the same `ReviewStore`,
            // and every single append must survive -- proving the
            // read-modify-write path serializes correctly instead of
            // silently overwriting a concurrent writer's update.
            let (_temp, store, session_ref) = store_with_session();
            let thread = store
                .add_thread(
                    &session_ref,
                    AddThreadRequest {
                        target: CommentTarget::Review,
                        body: "root".to_string(),
                        author: ThreadAuthor::human("alice"),
                    },
                )
                .unwrap();
            let thread_id = thread.id().clone();
            let store = std::sync::Arc::new(store);

            const WRITERS: usize = 8;
            let handles: Vec<_> = (0..WRITERS)
                .map(|i| {
                    let store = std::sync::Arc::clone(&store);
                    let session_ref = session_ref.clone();
                    let thread_id = thread_id.clone();
                    std::thread::spawn(move || {
                        store
                            .reply_to_thread(
                                &session_ref,
                                &thread_id,
                                ThreadAuthor::agent(format!("writer-{i}")),
                                format!("reply {i}"),
                            )
                            .unwrap();
                    })
                })
                .collect();
            for handle in handles {
                handle.join().unwrap();
            }

            let reloaded = store.get_review(&session_ref).unwrap();
            let persisted = reloaded.find_thread(&thread_id).unwrap();
            // 1 root + one reply per writer thread; a lost update would
            // show up as fewer than WRITERS + 1 comments.
            assert_eq!(persisted.thread.comments().len(), WRITERS + 1);
            let bodies: std::collections::HashSet<_> = persisted
                .thread
                .replies()
                .map(|reply| reply.body.clone())
                .collect();
            for i in 0..WRITERS {
                assert!(
                    bodies.contains(&format!("reply {i}")),
                    "writer {i}'s reply must not be lost to a concurrent overwrite"
                );
            }
        }

        #[test]
        fn should_reject_thread_on_unregistered_file() {
            let (_temp, store, session_ref) = store_with_session();

            let err = store
                .add_thread(
                    &session_ref,
                    AddThreadRequest {
                        target: CommentTarget::File {
                            path: PathBuf::from("missing.rs"),
                        },
                        body: "note".to_string(),
                        author: ThreadAuthor::human("alice"),
                    },
                )
                .unwrap_err();

            assert!(matches!(err, TuicrError::InvalidInput(_)));
        }

        #[test]
        fn should_reply_resolve_and_reopen_thread_through_store() {
            let (_temp, store, session_ref) = store_with_session();
            let thread = store
                .add_thread(
                    &session_ref,
                    AddThreadRequest {
                        target: CommentTarget::Review,
                        body: "root".to_string(),
                        author: ThreadAuthor::human("alice"),
                    },
                )
                .unwrap();
            let thread_id = thread.id().clone();

            let after_reply = store
                .reply_to_thread(
                    &session_ref,
                    &thread_id,
                    ThreadAuthor::agent("copilot"),
                    "ack",
                )
                .unwrap();
            assert_eq!(after_reply.thread.comments().len(), 2);

            let resolved = store.resolve_thread(&session_ref, &thread_id).unwrap();
            assert!(resolved);
            let reloaded = store.get_review(&session_ref).unwrap();
            assert_eq!(
                reloaded.find_thread(&thread_id).unwrap().thread.status(),
                ThreadStatus::Resolved
            );

            let reopened = store.reopen_thread(&session_ref, &thread_id).unwrap();
            assert!(reopened);
            let reloaded = store.get_review(&session_ref).unwrap();
            assert_eq!(
                reloaded.find_thread(&thread_id).unwrap().thread.status(),
                ThreadStatus::Open
            );
        }

        #[test]
        fn should_dismiss_thread_terminally_through_store() {
            let (_temp, store, session_ref) = store_with_session();
            let thread = store
                .add_thread(
                    &session_ref,
                    AddThreadRequest {
                        target: CommentTarget::Review,
                        body: "won't fix".to_string(),
                        author: ThreadAuthor::human("alice"),
                    },
                )
                .unwrap();
            let thread_id = thread.id().clone();

            store.dismiss_thread(&session_ref, &thread_id).unwrap();
            let resolved_after_dismiss = store.resolve_thread(&session_ref, &thread_id).unwrap();
            let reopened_after_dismiss = store.reopen_thread(&session_ref, &thread_id).unwrap();

            assert!(!resolved_after_dismiss);
            assert!(!reopened_after_dismiss);
            let reloaded = store.get_review(&session_ref).unwrap();
            assert_eq!(
                reloaded.find_thread(&thread_id).unwrap().thread.status(),
                ThreadStatus::Dismissed
            );
        }

        #[test]
        fn should_upsert_provider_mapping_idempotently_through_store() {
            let (_temp, store, session_ref) = store_with_session();
            let thread = store
                .add_thread(
                    &session_ref,
                    AddThreadRequest {
                        target: CommentTarget::Review,
                        body: "imported thread".to_string(),
                        author: ThreadAuthor::remote("github-user", "gh-author-1"),
                    },
                )
                .unwrap();
            let thread_id = thread.id().clone();

            store
                .upsert_thread_provider_mapping(
                    &session_ref,
                    &thread_id,
                    "github",
                    serde_json::json!({"id": "PRRT_1"}),
                )
                .unwrap();
            let mapped_again = store
                .upsert_thread_provider_mapping(
                    &session_ref,
                    &thread_id,
                    "github",
                    serde_json::json!({"id": "PRRT_1"}),
                )
                .unwrap();

            assert_eq!(mapped_again.provider_mappings.len(), 1);
            assert!(mapped_again.has_provider_id("github", "PRRT_1"));

            let reloaded = store.get_review(&session_ref).unwrap();
            assert!(
                reloaded
                    .find_thread_by_provider("github", "PRRT_1")
                    .is_some()
            );
        }

        #[test]
        fn should_import_remote_thread_idempotently_by_composing_find_and_add() {
            // Migration-audit §5: no HTTP/remote-mutation pipeline exists in
            // this ticket, so there is no production "import" call site yet
            // -- but the two primitives a future importer must compose
            // (`find_thread_by_provider` to check for an existing mapping,
            // `add_thread` + `upsert_thread_provider_mapping` to create one)
            // must themselves compose into an idempotent find-or-create.
            // This simulates importing the *same* fixture fetch twice and
            // asserts the thread count never grows past one.
            let (_temp, store, session_ref) = store_with_session();

            fn import_once(
                store: &ReviewStore,
                session_ref: &SessionRef,
                provider: &str,
                provider_id: &str,
                body: &str,
                author_name: &str,
            ) -> PersistedThread {
                let session = store.get_review(session_ref).unwrap();
                if let Some(existing) = session.find_thread_by_provider(provider, provider_id) {
                    return existing.clone();
                }
                let thread = store
                    .add_thread(
                        session_ref,
                        AddThreadRequest {
                            target: CommentTarget::Review,
                            body: body.to_string(),
                            author: ThreadAuthor::remote(author_name, "gh-author-1"),
                        },
                    )
                    .unwrap();
                store
                    .upsert_thread_provider_mapping(
                        session_ref,
                        thread.id(),
                        provider,
                        serde_json::json!({"id": provider_id}),
                    )
                    .unwrap()
            }

            let first = import_once(
                &store,
                &session_ref,
                "github",
                "PRRT_imported_1",
                "please fix this",
                "github-user",
            );
            let second = import_once(
                &store,
                &session_ref,
                "github",
                "PRRT_imported_1",
                "please fix this",
                "github-user",
            );

            assert_eq!(
                first.id(),
                second.id(),
                "re-importing the same provider ID must find the same thread, not mint a new one"
            );
            let reloaded = store.get_review(&session_ref).unwrap();
            assert_eq!(
                reloaded.threads().len(),
                1,
                "importing the same fixture fetch twice must not grow the thread count"
            );
            assert_eq!(
                reloaded
                    .threads()
                    .iter()
                    .filter(|t| t.has_provider_id("github", "PRRT_imported_1"))
                    .count(),
                1
            );
        }

        #[test]
        fn should_refresh_thread_anchor_via_store_with_unique_context_relocation() {
            let (_temp, store, session_ref) = store_with_session();
            let mut session = store.get_review(&session_ref).unwrap();
            let path = PathBuf::from("src/main.rs");
            let original_lines: Vec<&str> = vec!["one", "two", "three", "four"];
            let context = crate::model::AnchorContext::capture(&original_lines, 1, 1, 1).unwrap();
            let anchor =
                Anchor::line_with_context("src/main.rs", crate::model::AnchorSide::New, 2, context)
                    .unwrap();
            session.add_thread(
                anchor,
                crate::model::ThreadComment::new(ThreadAuthor::human("alice"), "about 'two'"),
            );
            store.save_review(&session).unwrap();

            // Insert a new line before "one", shifting "two" down by one
            // without disturbing its captured before/after context.
            let new_lines: Vec<String> = vec![
                "zero".to_string(),
                "one".to_string(),
                "two".to_string(),
                "three".to_string(),
                "four".to_string(),
            ];
            let mut file_lines = HashMap::new();
            file_lines.insert(path.clone(), new_lines);

            let results = store
                .refresh_thread_anchors(&session_ref, &file_lines, &HashMap::new())
                .unwrap();
            assert_eq!(results.len(), 1);

            let reloaded = store.get_review(&session_ref).unwrap();
            let thread = &reloaded.threads()[0];
            assert_eq!(thread.thread.anchor().state(), AnchorState::Current);
            match thread.thread.anchor().target() {
                crate::model::AnchorTarget::Line { line, .. } => assert_eq!(*line, 3),
                other => panic!("expected Line anchor, got {other:?}"),
            }
        }

        #[test]
        fn should_relocate_thread_current_when_an_unrelated_line_changes_elsewhere_in_the_file() {
            // Mirrors the migration-audit finding on `file_review_carried_forward`
            // (src/app/session.rs): that TUI-layer helper drops every draft
            // in a file outright the moment the file's content hash changes
            // at all, with no per-anchor relocation. This test proves the
            // STORE-level primitive the next wave should wire in instead
            // does not have that all-or-nothing failure mode: a change to
            // a line far away from a thread's anchor must relocate that
            // thread as `Current` at its original line, never drop it.
            let (_temp, store, session_ref) = store_with_session();
            let mut session = store.get_review(&session_ref).unwrap();
            let path = PathBuf::from("src/main.rs");
            let original_lines: Vec<&str> = vec!["fn main() {", "let x = 1;", "let y = 2;", "}"];
            let context = crate::model::AnchorContext::capture(&original_lines, 1, 1, 1).unwrap();
            let anchor =
                Anchor::line_with_context("src/main.rs", crate::model::AnchorSide::New, 2, context)
                    .unwrap();
            session.add_thread(
                anchor,
                crate::model::ThreadComment::new(ThreadAuthor::human("alice"), "about x"),
            );
            store.save_review(&session).unwrap();

            // An unrelated line, far from the anchor, changes content --
            // the anchor's own line and its surrounding context are
            // untouched, so its line number must not shift either.
            let new_lines: Vec<String> = vec![
                "fn main() {".to_string(),
                "let x = 1;".to_string(),
                "let y = 2;".to_string(),
                "println!(\"unrelated edit\");".to_string(),
            ];
            let mut file_lines = HashMap::new();
            file_lines.insert(path, new_lines);

            let results = store
                .refresh_thread_anchors(&session_ref, &file_lines, &HashMap::new())
                .unwrap();
            assert_eq!(results.len(), 1);
            assert!(matches!(
                results[0].1,
                ThreadAnchorRefresh::Applied(AnchorRelocation::Current { .. })
            ));

            let reloaded = store.get_review(&session_ref).unwrap();
            let thread = &reloaded.threads()[0];
            assert_eq!(
                thread.thread.anchor().state(),
                AnchorState::Current,
                "an unrelated edit elsewhere in the file must never drop or stale a thread"
            );
            assert_eq!(
                thread.thread.root().unwrap().body,
                "about x",
                "the comment itself must be fully preserved, not dropped"
            );
            match thread.thread.anchor().target() {
                crate::model::AnchorTarget::Line { line, .. } => assert_eq!(*line, 2),
                other => panic!("expected Line anchor, got {other:?}"),
            }
        }

        #[test]
        fn should_leave_legacy_migrated_no_context_anchor_relocation_inert() {
            // Legacy `Comment`s never captured before/after context lines,
            // so every thread produced by `migrate_legacy_comments_to_
            // threads` has `Anchor.context: None` (see
            // `thread_from_legacy_comment`). Per the frozen contract's own
            // doc comment on `Anchor::relocate_with_remap` -- "anchors with
            // no captured context or remap are left untouched" -- refreshing
            // such an anchor must always report `Current` at its original
            // line, completely independent of the new file content, rather
            // than being marked `Stale`/`Ambiguous` or silently relocated.
            // This proves that documented behavior holds end-to-end through
            // the store for a real legacy-migrated thread.
            let (_temp, store, session_ref) = store_with_session();
            let mut session = store.get_review(&session_ref).unwrap();
            session.version = "1.3".to_string();
            let path = PathBuf::from("src/main.rs");
            session.get_file_mut(&path).unwrap().add_line_comment(
                2,
                Comment::new("about x".to_string(), CommentType::None, None),
            );
            session.migrate_legacy_comments_to_threads();
            assert_eq!(session.threads().len(), 1);
            assert!(
                session.threads()[0].thread.anchor().context.is_none(),
                "legacy-migrated line anchors must have no captured context"
            );
            store.save_review(&session).unwrap();

            // Completely different content, and even shorter than the
            // original anchor's line number -- a context-aware anchor
            // would go `Stale` here (no unique match, or out of range);
            // a no-context anchor must not even look.
            let new_lines: Vec<String> = vec!["totally different content".to_string()];
            let mut file_lines = HashMap::new();
            file_lines.insert(path, new_lines);

            let results = store
                .refresh_thread_anchors(&session_ref, &file_lines, &HashMap::new())
                .unwrap();
            assert_eq!(results.len(), 1);
            assert!(matches!(
                results[0].1,
                ThreadAnchorRefresh::Applied(AnchorRelocation::Current { new_start: 2, .. })
            ));

            let reloaded = store.get_review(&session_ref).unwrap();
            let thread = &reloaded.threads()[0];
            assert_eq!(thread.thread.anchor().state(), AnchorState::Current);
            match thread.thread.anchor().target() {
                crate::model::AnchorTarget::Line { line, .. } => assert_eq!(*line, 2),
                other => panic!("expected Line anchor, got {other:?}"),
            }
        }

        #[test]
        fn should_freeze_dismissed_thread_anchor_across_a_real_json_save_and_reload() {
            // Migration-audit §9 item 10: a Dismissed thread's anchor freeze
            // (thread.rs's own in-memory `lifecycle_tests`/`freeze_tests`)
            // must also survive a *real* persistence round trip -- not just
            // in-memory mutation within one process -- and `refresh_thread_
            // anchors` must still treat it as frozen even when the anchor
            // has a path/line that would otherwise be touched.
            let (_temp, store, session_ref) = store_with_session();
            let path = PathBuf::from("src/main.rs");
            let thread = store
                .add_thread(
                    &session_ref,
                    AddThreadRequest {
                        target: CommentTarget::Line {
                            path: path.clone(),
                            line: 2,
                            side: LineSide::New,
                        },
                        body: "won't fix, dismissing".to_string(),
                        author: ThreadAuthor::human("alice"),
                    },
                )
                .unwrap();
            let thread_id = thread.id().clone();
            store.dismiss_thread(&session_ref, &thread_id).unwrap();

            // Force a real JSON round trip: `get_review` always reads and
            // deserializes the file from disk (no in-memory cache), so
            // reusing the same store handle still exercises the exact save
            // → JSON → reload path, not an in-process-only mutation.
            let before = store.get_review(&session_ref).unwrap();
            let before_thread = before.find_thread(&thread_id).unwrap().clone();
            assert_eq!(before_thread.thread.status(), ThreadStatus::Dismissed);

            // Content that would relocate/stale a *live* line anchor.
            let new_lines: Vec<String> = vec!["completely rewritten".to_string()];
            let mut file_lines = HashMap::new();
            file_lines.insert(path, new_lines);
            let results = store
                .refresh_thread_anchors(&session_ref, &file_lines, &HashMap::new())
                .unwrap();
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].1, ThreadAnchorRefresh::Frozen);

            let after = store.get_review(&session_ref).unwrap();
            let after_thread = after.find_thread(&thread_id).unwrap().clone();
            assert_eq!(
                after_thread.thread.anchor(),
                before_thread.thread.anchor(),
                "a dismissed thread's anchor must be byte-identical after refresh_thread_anchors, \
                 across a real save/reload, not just in-process"
            );
            assert_eq!(after_thread.thread.status(), ThreadStatus::Dismissed);

            // Terminal: reopening a dismissed thread is a permanent no-op,
            // even after the real persistence round trip above.
            assert!(!store.reopen_thread(&session_ref, &thread_id).unwrap());
            let still_dismissed = store.get_review(&session_ref).unwrap();
            assert_eq!(
                still_dismissed
                    .find_thread(&thread_id)
                    .unwrap()
                    .thread
                    .status(),
                ThreadStatus::Dismissed
            );
        }

        #[test]
        fn should_freeze_anchor_refresh_for_resolved_thread() {
            let (_temp, store, session_ref) = store_with_session();
            let thread = store
                .add_thread(
                    &session_ref,
                    AddThreadRequest {
                        target: CommentTarget::Review,
                        body: "resolved already".to_string(),
                        author: ThreadAuthor::human("alice"),
                    },
                )
                .unwrap();
            let thread_id = thread.id().clone();
            store.resolve_thread(&session_ref, &thread_id).unwrap();

            let mut remaps = HashMap::new();
            remaps.insert(thread_id.clone(), ProviderRemap::line(99));
            let results = store
                .refresh_thread_anchors(&session_ref, &HashMap::new(), &remaps)
                .unwrap();

            // Review anchors have no path, so refresh_thread_anchors skips
            // them entirely regardless of status; assert no crash and the
            // thread stays Resolved either way.
            assert!(results.is_empty());
            let reloaded = store.get_review(&session_ref).unwrap();
            assert_eq!(
                reloaded.find_thread(&thread_id).unwrap().thread.status(),
                ThreadStatus::Resolved
            );
        }

        #[test]
        fn should_migrate_legacy_session_on_load_and_persist_only_after_explicit_save() {
            let temp = tempfile::tempdir().unwrap();
            let repo = temp.path().join("repo");
            std::fs::create_dir_all(&repo).unwrap();
            let reviews_dir = temp.path().join("reviews");
            let store = ReviewStore::with_reviews_dir(reviews_dir.clone());

            let mut session = test_session(repo);
            session.version = "1.3".to_string();
            session.review_comments.push(Comment::new(
                "legacy note".to_string(),
                CommentType::None,
                None,
            ));
            let session_ref =
                crate::persistence::storage::save_session_in_dir(&session, &reviews_dir)
                    .map(SessionRef::from_path)
                    .unwrap();

            // Loading migrates in-memory (deterministically, idempotently)
            // but does not itself rewrite the v1.3 file on disk.
            let loaded_once = store.get_review(&session_ref).unwrap();
            assert_eq!(loaded_once.threads().len(), 1);
            let raw_contents = std::fs::read_to_string(session_ref.path()).unwrap();
            let on_disk_untouched: ReviewSession = serde_json::from_str(&raw_contents).unwrap();
            assert_eq!(on_disk_untouched.version, "1.3");

            // Explicitly saving the migrated session persists it.
            store.save_review(&loaded_once).unwrap();
            let reloaded = store.get_review(&session_ref).unwrap();
            assert_eq!(reloaded.version, CURRENT_SESSION_VERSION);
            assert_eq!(reloaded.threads().len(), 1);
            assert_eq!(reloaded.review_comments.len(), 1, "legacy field preserved");

            // Loading a third time must not duplicate the migrated thread.
            let reloaded_again = store.get_review(&session_ref).unwrap();
            assert_eq!(reloaded_again.threads().len(), 1);
        }

        #[test]
        fn should_surface_a_legacy_review_add_via_list_threads_on_an_already_current_session() {
            // Blocking-issue regression: `review add` on a session already
            // at CURRENT_SESSION_VERSION only ever touches legacy fields
            // (`add_comment_to_session` never calls `add_thread`), so
            // without an incremental (not version-gated) sync inside
            // `migrate_legacy_comments_to_threads`, a subsequent `review
            // thread list` would never surface it -- the old version-gated
            // migration was a permanent no-op once a session reached 1.4.
            let (_temp, store, session_ref) = store_with_session();
            assert_eq!(
                store.get_review(&session_ref).unwrap().version,
                CURRENT_SESSION_VERSION,
                "sanity: this session is already at the current version, not a legacy 1.3 one"
            );

            store
                .add_comment(
                    &session_ref,
                    AddCommentRequest {
                        target: CommentTarget::Line {
                            path: PathBuf::from("src/main.rs"),
                            line: 5,
                            side: LineSide::New,
                        },
                        content: "please add a test".to_string(),
                        comment_type: CommentType::from_id("issue"),
                        author: crate::model::comment::DEFAULT_AUTHOR.to_string(),
                        commit_id: None,
                    },
                )
                .unwrap();

            // A separate `review thread list` invocation (fresh load) must
            // see the newly-added legacy comment mirrored as a thread.
            let threads = store.list_threads(&session_ref).unwrap();
            assert_eq!(threads.len(), 1);
            assert_eq!(threads[0].thread.root().unwrap().body, "please add a test");

            // The legacy view must still work unchanged (authorship visible
            // side by side, per required behavior #5).
            let reloaded = store.get_review(&session_ref).unwrap();
            let line_comments = &reloaded.files[&PathBuf::from("src/main.rs")].line_comments[&5];
            assert_eq!(line_comments.len(), 1);
            assert_eq!(line_comments[0].content, "please add a test");

            // Listing threads again (yet another fresh load) must not
            // duplicate the mirrored thread.
            let threads_again = store.list_threads(&session_ref).unwrap();
            assert_eq!(threads_again.len(), 1);
        }
    }
}
