use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use super::comment::{Comment, LineRange};
use super::diff_types::{DiffFile, FileStatus};
use super::thread::{Anchor, AnchorSide, ThreadComment, ThreadId};
use super::thread_store::{self, CURRENT_SESSION_VERSION, PersistedThread};
use crate::forge::remote_comments::PrCommentsVisibility;
use crate::forge::traits::PrSessionKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClearScope {
    CommentsOnly,
    CommentsAndReviewed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReview {
    pub path: PathBuf,
    pub reviewed: bool,
    pub status: FileStatus,
    pub file_comments: Vec<Comment>,
    pub line_comments: HashMap<u32, Vec<Comment>>,
    #[serde(default)]
    pub reviewed_hunks: BTreeSet<String>,
    #[serde(default)]
    pub content_hash: Option<u64>,
}

impl FileReview {
    pub fn new(path: PathBuf, status: FileStatus, content_hash: u64) -> Self {
        Self {
            path,
            reviewed: false,
            status,
            file_comments: Vec::new(),
            line_comments: HashMap::new(),
            reviewed_hunks: BTreeSet::new(),
            content_hash: Some(content_hash),
        }
    }

    pub fn comment_count(&self) -> usize {
        self.file_comments.len() + self.line_comments.values().map(|v| v.len()).sum::<usize>()
    }

    pub fn add_file_comment(&mut self, comment: Comment) {
        self.file_comments.push(comment);
    }

    pub fn add_line_comment(&mut self, line: u32, comment: Comment) {
        self.line_comments.entry(line).or_default().push(comment);
    }

    pub fn toggle_hunk_reviewed(&mut self, key: String) -> bool {
        if self.reviewed_hunks.contains(&key) {
            self.reviewed_hunks.remove(&key);
            false
        } else {
            self.reviewed_hunks.insert(key);
            true
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SessionDiffSource {
    #[default]
    WorkingTree,
    Staged,
    Unstaged,
    StagedAndUnstaged,
    CommitRange,
    WorkingTreeAndCommits,
    StagedUnstagedAndCommits,
    /// Remote pull request review. Per-PR identity lives in
    /// `ReviewSession::pr_session_key`; this variant is a discriminator so
    /// the persistence layer can route to PR-specific filename construction.
    PullRequest,
    /// Whole-repo annotation surface. Every tracked file is shown in
    /// context-only rendering, sourced from `git ls-files`. The persisted
    /// `base_commit` for these sessions starts with `"pristine:"` so the
    /// reload path can match by prefix instead of exact HEAD.
    Pristine,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewSession {
    pub id: String,
    pub version: String,
    pub repo_path: PathBuf,
    #[serde(default)]
    pub branch_name: Option<String>,
    pub base_commit: String,
    #[serde(default)]
    pub diff_source: SessionDiffSource,
    #[serde(default)]
    pub commit_range: Option<Vec<String>>,
    /// Identity for PR-mode sessions. `None` for local sessions. Default is
    /// `None` so existing local session JSON deserializes unchanged.
    #[serde(default)]
    pub pr_session_key: Option<PrSessionKey>,
    /// Per-session visibility setting for existing remote forge comments.
    /// Only meaningful in PR mode. Defaults to `Unresolved` so a fresh PR
    /// session — or a session saved before this field existed — shows
    /// unresolved threads.
    #[serde(default)]
    pub remote_comments_visibility: PrCommentsVisibility,
    /// Persisted inline commit selector range for PR sessions. Indices
    /// reference the per-head-SHA `pr_commits` list captured at open
    /// time. `None` means "all commits" (or no selector). Older sessions
    /// without this field deserialize as `None`.
    #[serde(default)]
    pub commit_selection_range: Option<(usize, usize)>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub review_comments: Vec<Comment>,
    pub files: HashMap<PathBuf, FileReview>,
    pub session_notes: Option<String>,
    /// Durable provider-neutral threads. Populated directly for sessions
    /// created at `CURRENT_SESSION_VERSION` or later; migrated
    /// deterministically from `review_comments`/`files[..].*_comments` for
    /// older sessions the first time they load (see
    /// [`Self::migrate_legacy_comments_to_threads`]). Legacy comment fields
    /// are never removed by migration, so old public API/JSON consumers
    /// keep working unchanged.
    #[serde(default)]
    pub threads: Vec<PersistedThread>,
}

impl ReviewSession {
    pub fn new(
        repo_path: PathBuf,
        base_commit: String,
        branch_name: Option<String>,
        diff_source: SessionDiffSource,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            version: CURRENT_SESSION_VERSION.to_string(),
            repo_path,
            branch_name,
            base_commit,
            diff_source,
            commit_range: None,
            pr_session_key: None,
            remote_comments_visibility: PrCommentsVisibility::default(),
            commit_selection_range: None,
            created_at: now,
            updated_at: now,
            review_comments: Vec::new(),
            files: HashMap::new(),
            session_notes: None,
            threads: Vec::new(),
        }
    }

    pub fn reviewed_count(&self) -> usize {
        self.files.values().filter(|f| f.reviewed).count()
    }

    pub fn has_reviewed_state(&self) -> bool {
        self.files
            .values()
            .any(|file| file.reviewed || !file.reviewed_hunks.is_empty())
    }

    /// Registers a file in the session. Returns true if the file was previously
    /// reviewed but its content changed, causing reviewed status to be reset.
    pub fn add_file(&mut self, path: PathBuf, status: FileStatus, content_hash: u64) -> bool {
        if let Some(review) = self.files.get_mut(&path) {
            let old_hash = review.content_hash;
            review.content_hash = Some(content_hash);
            if review.reviewed && old_hash != Some(content_hash) {
                review.reviewed = false;
                return true;
            }
            return false;
        }
        self.files
            .insert(path.clone(), FileReview::new(path, status, content_hash));
        false
    }

    pub fn add_diff_file(&mut self, file: &DiffFile) -> bool {
        let path = file.display_path().clone();
        let invalidated = self.add_file(path.clone(), file.status, file.content_hash);
        if let Some(review) = self.files.get_mut(&path) {
            let valid_hunks: BTreeSet<_> = file.hunk_review_keys().into_iter().collect();
            review
                .reviewed_hunks
                .retain(|key| valid_hunks.contains(key));
        }
        invalidated
    }

    /// Register a transient filtered diff without dropping hunk keys that
    /// belong to the broader persisted review scope.
    pub fn add_diff_file_preserving_hunks(&mut self, file: &DiffFile) -> bool {
        self.add_file(file.display_path().clone(), file.status, file.content_hash)
    }

    pub fn get_file_mut(&mut self, path: &PathBuf) -> Option<&mut FileReview> {
        self.files.get_mut(path)
    }

    pub fn has_comments(&self) -> bool {
        !self.review_comments.is_empty() || self.files.values().any(|f| f.comment_count() > 0)
    }

    pub fn clear_comments(&mut self, scope: ClearScope) -> (usize, usize) {
        let mut cleared = self.review_comments.len();
        let mut unreviewed = 0;
        self.review_comments.clear();
        for file in self.files.values_mut() {
            cleared += file.comment_count();
            file.file_comments.clear();
            file.line_comments.clear();
            if scope == ClearScope::CommentsAndReviewed {
                if file.reviewed || !file.reviewed_hunks.is_empty() {
                    unreviewed += 1;
                }
                file.reviewed = false;
                file.reviewed_hunks.clear();
            }
        }
        (cleared, unreviewed)
    }

    pub fn is_file_reviewed(&self, path: &PathBuf) -> bool {
        self.files.get(path).map(|r| r.reviewed).unwrap_or(false)
    }

    pub fn is_hunk_reviewed(&self, path: &PathBuf, key: &str) -> bool {
        self.files
            .get(path)
            .is_some_and(|review| review.reviewed_hunks.contains(key))
    }

    pub fn threads(&self) -> &[PersistedThread] {
        &self.threads
    }

    pub fn find_thread(&self, id: &ThreadId) -> Option<&PersistedThread> {
        self.threads.iter().find(|thread| thread.id() == id)
    }

    pub fn find_thread_mut(&mut self, id: &ThreadId) -> Option<&mut PersistedThread> {
        self.threads.iter_mut().find(|thread| thread.id() == id)
    }

    /// Find the thread that mirrors a legacy `Comment` with `comment_id`,
    /// whether it is that thread's root (the common case for
    /// `review_comments`/`file_comments`, which never group — see
    /// [`Self::migrate_legacy_comments_to_threads`]) or one of its replies
    /// (the common case for `line_comments` sharing one anchor). Lets TUI
    /// thread interactions (reply/resolve/reopen/dismiss keybindings) map a
    /// cursor position over a legacy comment onto its durable `ThreadId`
    /// without needing to re-derive the anchor/grouping the migration used.
    pub fn find_thread_by_legacy_comment_id(&self, comment_id: &str) -> Option<&PersistedThread> {
        self.threads.iter().find(|thread| {
            thread
                .thread
                .comments()
                .iter()
                .any(|c| c.id().as_str() == comment_id)
        })
    }

    /// Mutable counterpart of [`Self::find_thread_by_legacy_comment_id`].
    pub fn find_thread_by_legacy_comment_id_mut(
        &mut self,
        comment_id: &str,
    ) -> Option<&mut PersistedThread> {
        self.threads.iter_mut().find(|thread| {
            thread
                .thread
                .comments()
                .iter()
                .any(|c| c.id().as_str() == comment_id)
        })
    }

    /// Whether `id` is the id of some legacy `Comment` still tracked by
    /// this session (`review_comments`, any file's `file_comments`, or any
    /// file's `line_comments`) — i.e. it has an existing rendering via the
    /// legacy comment-box path, so a durable [`crate::model::thread::ThreadComment`]
    /// carrying this same id (see [`Self::migrate_legacy_comments_to_threads`],
    /// which reuses legacy `Comment.id`s verbatim) must not be rendered a
    /// second time. Any thread comment whose id this returns `false` for has
    /// no legacy shadow — most commonly a reply added natively via the
    /// TUI's thread-reply keybinding — and needs its own rendering path
    /// (see `format_thread_native_reply_lines` in `ui::comment_panel`).
    pub fn is_legacy_comment_id(&self, id: &str) -> bool {
        self.review_comments.iter().any(|c| c.id == id)
            || self.files.values().any(|file| {
                file.file_comments.iter().any(|c| c.id == id)
                    || file
                        .line_comments
                        .values()
                        .any(|comments| comments.iter().any(|c| c.id == id))
            })
    }

    /// Whether `comment_id` is the *last* legacy-mirrored comment (in the
    /// migrated durable thread's own chronological `comments()` order)
    /// belonging to its thread — i.e. the single correct place to splice/
    /// append/print that thread's native-only replies (no legacy `Comment`
    /// counterpart) during render or export, regardless of how many other
    /// same-anchor legacy comments were grouped into the same thread by
    /// [`Self::migrate_legacy_comments_to_threads`].
    ///
    /// A thread's `comments()` are always in the order they were added:
    /// root, then any legacy-mirrored replies grouped at the same anchor
    /// (in the order `migrate_legacy_comments_to_threads` synced them),
    /// then any native-only replies (always appended after via
    /// `Thread::reply`, since a native reply can only be added once the
    /// thread already exists). So the last legacy id in that order is
    /// always immediately followed only by native-only replies (if any) —
    /// making it the correct splice point.
    ///
    /// This is the single shared predicate every render/export path
    /// (`App::splice_native_thread_replies`, `ui::diff_view::push_native_thread_replies`,
    /// and `output::markdown`'s remote/local export loops) must use to
    /// decide *where* to emit a thread's native-only replies, so the three
    /// can never independently drift into different orderings for a
    /// grouped multi-comment thread. Returns `false` when `comment_id` has
    /// no thread at all (nothing to splice).
    pub fn is_last_legacy_comment_for_thread(&self, comment_id: &str) -> bool {
        let Some(persisted) = self.find_thread_by_legacy_comment_id(comment_id) else {
            return false;
        };
        persisted
            .thread
            .comments()
            .iter()
            .rev()
            .find(|c| self.is_legacy_comment_id(c.id().as_str()))
            .is_some_and(|c| c.id().as_str() == comment_id)
    }

    /// Find a thread previously imported/mapped from `provider` under
    /// `provider_id`. Used by (future) provider adapters to make repeated
    /// import/sync passes idempotent instead of creating duplicate threads.
    pub fn find_thread_by_provider(
        &self,
        provider: &str,
        provider_id: &str,
    ) -> Option<&PersistedThread> {
        self.threads
            .iter()
            .find(|thread| thread.has_provider_id(provider, provider_id))
    }

    /// Open a new durable thread anchored at `anchor` with `root` as its
    /// first comment. Returns the new thread's stable ID.
    pub fn add_thread(&mut self, anchor: Anchor, root: ThreadComment) -> ThreadId {
        let thread = super::thread::Thread::open(anchor, root);
        let id = thread.id().clone();
        self.threads.push(PersistedThread::new(thread));
        self.updated_at = Utc::now();
        id
    }

    /// Idempotently import fetched `remote_threads` (from `provider`, e.g.
    /// `"github"`/`"gitlab"`/`"gitea"`/`"forgejo"` — see
    /// [`crate::forge::traits::ForgeKind::provider_key`]) as durable
    /// [`PersistedThread`]s, preserving root/reply order, comment IDs,
    /// authors, and resolved/outdated state (see
    /// [`thread_store::thread_from_remote`]).
    ///
    /// Each remote thread converts to a [`PersistedThread`] whose ID is
    /// derived solely from `(provider, remote.id)`
    /// ([`thread_store::remote_thread_id_for`]), so re-importing the exact
    /// same remote threads (e.g. re-fetching twice, or a session
    /// reload) **merges into** the existing entry in place
    /// ([`thread_store::merge_remote_thread_into_existing`]) rather than
    /// appending a duplicate *or* naively overwriting it wholesale: a
    /// locally-added reply, a local resolve/dismiss, and a locally
    /// computed Stale/Ambiguous anchor relocation all survive a re-fetch
    /// untouched, while the provider's own root/reply edits and
    /// resolved/outdated flags still land from the fresh fetch. Threads
    /// with no comments are skipped (a thread always needs a root).
    /// Returns the number of *newly* created threads (merges into an
    /// already-imported thread are not counted, mirroring
    /// [`Self::migrate_legacy_comments_to_threads`]'s convention).
    pub fn import_remote_review_threads(
        &mut self,
        provider: &str,
        remote_threads: &[crate::forge::remote_comments::RemoteReviewThread],
    ) -> usize {
        let mut created = 0;
        for remote in remote_threads {
            if remote.comments.is_empty() {
                continue;
            }
            let persisted = thread_store::thread_from_remote(provider, remote);
            match self
                .threads
                .iter()
                .position(|existing| existing.id() == persisted.id())
            {
                Some(index) => {
                    thread_store::merge_remote_thread_into_existing(
                        &mut self.threads[index],
                        persisted,
                        provider,
                    );
                }
                None => {
                    self.threads.push(persisted);
                    created += 1;
                }
            }
        }
        if created > 0 {
            self.updated_at = Utc::now();
        }
        created
    }

    /// Deterministically and idempotently sync this session's legacy
    /// `review_comments`/`files[..].file_comments`/`files[..].line_comments`
    /// into durable [`PersistedThread`]s.
    ///
    /// Unlike a one-shot "migrate on first load" pass, this is safe (and
    /// necessary) to call on *every* session load regardless of
    /// `self.version`: a plain `review add` on an already-
    /// [`CURRENT_SESSION_VERSION`] session only ever writes the legacy
    /// fields, so without an incremental catch-up pass here, newly-added
    /// legacy comments would never become visible via `review thread
    /// list`/`show`. Each legacy source is synced independently:
    ///
    /// - `review_comments`/`file_comments` are always "1 comment = 1
    ///   thread" (no reply concept at that granularity); a comment is only
    ///   ever mirrored once, keyed by its own deterministic thread id
    ///   (anchor + that comment's own legacy id), so re-syncing an
    ///   already-mirrored comment is a no-op and a brand-new comment gets
    ///   its own new thread.
    /// - `line_comments` sharing the exact same anchor (side + line_range)
    ///   are grouped into one thread (root + ordered replies). The
    ///   thread's deterministic id depends only on the anchor and the
    ///   *first* comment in the group, which never changes as more
    ///   comments are appended to the same line, so a previously-synced
    ///   group's thread is found again on every call; any comments in the
    ///   current group not yet present in that thread (by legacy id) are
    ///   appended as new ordered replies, and comments already present are
    ///   left untouched (never duplicated).
    ///
    /// Legacy comment fields are never mutated or removed — this only ever
    /// *adds* threads/replies — so existing public API/JSON consumers
    /// reading `review_comments`/`files` keep working unchanged.
    ///
    /// Ordering is deterministic: review-level comments first (in their
    /// existing order), then per-file comments in file-path order, then
    /// file-level comments followed by line/range comments in ascending
    /// line order. See [`thread_store::thread_from_legacy_comment`] for the
    /// author-kind migration heuristic. Returns the number of *new*
    /// threads created (incremental replies appended to already-existing
    /// threads are not counted, since no production caller inspects this
    /// return value — only tests do, to assert on first-sync counts).
    pub fn migrate_legacy_comments_to_threads(&mut self) -> usize {
        let mut created = 0;

        for comment in &self.review_comments {
            created +=
                Self::sync_single_comment_thread(&mut self.threads, Anchor::review(), comment);
        }

        let mut paths: Vec<_> = self.files.keys().cloned().collect();
        paths.sort();
        for path in paths {
            let path_str = path.to_string_lossy().to_string();
            let review = &self.files[&path];

            for comment in review.file_comments.clone() {
                created += Self::sync_single_comment_thread(
                    &mut self.threads,
                    Anchor::file(path_str.clone()),
                    &comment,
                );
            }

            let mut lines: Vec<_> = review.line_comments.keys().copied().collect();
            lines.sort();
            for line in lines {
                let comments = self.files[&path].line_comments[&line].clone();

                // Group comments that resolve to the *exact same* anchor
                // (identical side + line_range; a bare line comment and a
                // range comment ending on the same line number share this
                // HashMap bucket but are different anchors) so multiple
                // independent legacy notes at one spot become one thread
                // (root + ordered replies) instead of several one-comment
                // threads fragmenting a single anchor point. `Vec`-based
                // grouping (not a HashMap) preserves original insertion
                // order both within and across groups.
                let mut groups: Vec<(AnchorSide, Option<LineRange>, Vec<Comment>)> = Vec::new();
                for comment in comments {
                    let side = thread_store::anchor_side_for_legacy(comment.side);
                    match groups
                        .iter_mut()
                        .find(|(s, r, _)| *s == side && *r == comment.line_range)
                    {
                        Some((_, _, group)) => group.push(comment),
                        None => groups.push((side, comment.line_range, vec![comment])),
                    }
                }

                for (side, line_range, group_comments) in groups {
                    let anchor = match line_range {
                        Some(range) => {
                            Anchor::range(path_str.clone(), side, range.start, range.end)
                                .expect("LineRange is always normalized start <= end")
                        }
                        None => Anchor::line(path_str.clone(), side, line),
                    };
                    created +=
                        Self::sync_comment_group_thread(&mut self.threads, anchor, &group_comments);
                }
            }
        }

        self.version = CURRENT_SESSION_VERSION.to_string();
        created
    }

    /// Idempotently ensure a single legacy comment with no reply concept
    /// (`review_comments`/`file_comments`) is mirrored as its own thread.
    /// Returns `1` if a new thread was created, `0` if a thread for this
    /// exact anchor/comment-id combination already exists (a repeat sync
    /// of an already-migrated comment).
    ///
    /// Takes `threads: &mut Vec<PersistedThread>` rather than `&mut self`
    /// so callers can invoke it while separately holding a shared borrow
    /// of another field of `self` (e.g. `self.review_comments`/
    /// `self.files`) in the same loop.
    fn sync_single_comment_thread(
        threads: &mut Vec<PersistedThread>,
        anchor: Anchor,
        comment: &Comment,
    ) -> usize {
        let thread_id = thread_store::legacy_thread_id_for(&anchor, &comment.id);
        if threads.iter().any(|thread| *thread.id() == thread_id) {
            return 0;
        }
        threads.push(thread_store::thread_from_legacy_comment(anchor, comment));
        1
    }

    /// Idempotently ensure a group of same-anchor legacy `line_comments`
    /// are mirrored as one thread (root + ordered replies).
    ///
    /// If a thread for this exact anchor/root-comment combination already
    /// exists (from a previous sync), any comment in `comments` not yet
    /// present in that thread (matched by legacy `Comment.id`) is appended
    /// as a new ordered reply — this is what lets a brand-new comment
    /// added to an already-synced line become visible via `review thread
    /// list` without creating a duplicate thread or re-appending comments
    /// that were already migrated. Returns `1` only when a brand-new
    /// thread was created, `0` otherwise (even if replies were appended to
    /// an existing thread).
    fn sync_comment_group_thread(
        threads: &mut Vec<PersistedThread>,
        anchor: Anchor,
        comments: &[Comment],
    ) -> usize {
        let thread_id = thread_store::legacy_thread_id_for(&anchor, &comments[0].id);

        if let Some(existing) = threads.iter_mut().find(|thread| *thread.id() == thread_id) {
            let already_mirrored: std::collections::HashSet<&str> = existing
                .thread
                .comments()
                .iter()
                .map(|thread_comment| thread_comment.id().as_str())
                .collect();
            let new_replies: Vec<Comment> = comments
                .iter()
                .filter(|comment| !already_mirrored.contains(comment.id.as_str()))
                .cloned()
                .collect();
            for comment in &new_replies {
                existing
                    .thread
                    .reply(thread_store::legacy_reply_comment(comment));
            }
            return 0;
        }

        threads.push(thread_store::thread_from_legacy_comment_group(
            anchor, comments,
        ));
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::comment::{Comment, CommentType};
    use crate::model::{DiffHunk, DiffLine, LineOrigin};

    // Arbitrary hash value for tests that don't care about the specific hash.
    const SOME_HASH: u64 = 0xdeadbeef;

    fn test_session() -> ReviewSession {
        ReviewSession::new(
            PathBuf::from("/repo"),
            "abc123".to_string(),
            None,
            SessionDiffSource::WorkingTree,
        )
    }

    fn test_hunk(new_start: u32, content: &str) -> DiffHunk {
        DiffHunk {
            header: format!("@@ -{new_start},1 +{new_start},1 @@"),
            lines: vec![DiffLine {
                origin: LineOrigin::Context,
                content: content.to_string(),
                old_lineno: Some(new_start),
                new_lineno: Some(new_start),
                highlighted_spans: None,
            }],
            old_start: new_start,
            old_count: 1,
            new_start,
            new_count: 1,
        }
    }

    fn test_diff_file(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
        let content_hash = DiffFile::compute_content_hash(&hunks);
        DiffFile {
            old_path: None,
            new_path: Some(PathBuf::from(path)),
            status: FileStatus::Modified,
            hunks,
            is_binary: false,
            is_too_large: false,
            is_commit_message: false,
            content_hash,
        }
    }

    #[test]
    fn should_return_zero_when_clearing_empty_session() {
        let mut session = test_session();
        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 0);
        assert_eq!(unreviewed, 0);
    }

    #[test]
    fn should_clear_review_level_comments() {
        let mut session = test_session();
        session.review_comments.push(Comment::new(
            "note".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        session.review_comments.push(Comment::new(
            "issue".to_string(),
            CommentType::from_id("issue"),
            None,
        ));

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 2);
        assert_eq!(unreviewed, 0);
        assert!(session.review_comments.is_empty());
    }

    #[test]
    fn should_clear_file_and_line_comments() {
        let mut session = test_session();
        let path = PathBuf::from("src/main.rs");
        session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
        let file = session.get_file_mut(&path).unwrap();
        file.add_file_comment(Comment::new(
            "comment".to_string(),
            CommentType::from_id("note"),
            None,
        ));
        file.add_line_comment(
            10,
            Comment::new("line".to_string(), CommentType::from_id("note"), None),
        );

        let (cleared, _) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 2);

        let file = session.files.get(&path).unwrap();
        assert!(file.file_comments.is_empty());
        assert!(file.line_comments.is_empty());
    }

    #[test]
    fn should_reset_reviewed_status_on_all_files() {
        let mut session = test_session();
        let path_a = PathBuf::from("a.rs");
        let path_b = PathBuf::from("b.rs");
        session.add_file(path_a.clone(), FileStatus::Modified, SOME_HASH);
        session.add_file(path_b.clone(), FileStatus::Added, SOME_HASH);

        session.get_file_mut(&path_a).unwrap().reviewed = true;
        session.get_file_mut(&path_b).unwrap().reviewed = true;

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 0);
        assert_eq!(unreviewed, 2);
        assert!(!session.is_file_reviewed(&path_a));
        assert!(!session.is_file_reviewed(&path_b));
    }

    #[test]
    fn should_only_count_reviewed_files_as_unreviewed() {
        let mut session = test_session();
        let reviewed = PathBuf::from("reviewed.rs");
        let pending = PathBuf::from("pending.rs");
        session.add_file(reviewed.clone(), FileStatus::Modified, SOME_HASH);
        session.add_file(pending.clone(), FileStatus::Modified, SOME_HASH);

        session.get_file_mut(&reviewed).unwrap().reviewed = true;

        let (_, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(unreviewed, 1);
    }

    #[test]
    fn should_clear_both_comments_and_reviewed_status() {
        let mut session = test_session();
        let path = PathBuf::from("src/lib.rs");
        session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
        let file = session.get_file_mut(&path).unwrap();
        file.reviewed = true;
        file.add_file_comment(Comment::new(
            "comment".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        session.review_comments.push(Comment::new(
            "review".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);
        assert_eq!(cleared, 2);
        assert_eq!(unreviewed, 1);
        assert!(!session.is_file_reviewed(&path));
    }

    #[test]
    fn should_clear_hunk_reviewed_status() {
        let mut session = test_session();
        let file = test_diff_file("src/main.rs", vec![test_hunk(10, "same")]);
        let path = file.display_path().clone();
        let key = file.hunk_review_key(0).unwrap();

        session.add_diff_file(&file);
        session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(key.clone());

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsAndReviewed);

        assert_eq!(cleared, 0);
        assert_eq!(unreviewed, 1);
        assert!(!session.is_hunk_reviewed(&path, &key));
    }

    #[test]
    fn should_preserve_reviewed_status_when_requested() {
        let mut session = test_session();
        let path = PathBuf::from("src/lib.rs");
        session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
        let file = session.get_file_mut(&path).unwrap();
        file.reviewed = true;
        file.add_file_comment(Comment::new(
            "comment".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        session.review_comments.push(Comment::new(
            "review".to_string(),
            CommentType::from_id("note"),
            None,
        ));

        let (cleared, unreviewed) = session.clear_comments(ClearScope::CommentsOnly);
        assert_eq!(cleared, 2);
        assert_eq!(unreviewed, 0);
        assert!(session.is_file_reviewed(&path));
    }

    #[test]
    fn should_store_content_hash_on_new_file() {
        let mut session = test_session();
        let path = PathBuf::from("new.rs");
        session.add_file(path.clone(), FileStatus::Added, 42);

        let file = session.files.get(&path).unwrap();
        assert_eq!(file.content_hash, Some(42));
        assert!(!file.reviewed);
    }

    #[test]
    fn should_keep_reviewed_when_hash_unchanged() {
        let mut session = test_session();
        let path = PathBuf::from("stable.rs");
        session.add_file(path.clone(), FileStatus::Modified, 100);
        session.get_file_mut(&path).unwrap().reviewed = true;

        let invalidated = session.add_file(path.clone(), FileStatus::Modified, 100);
        assert!(!invalidated);
        assert!(session.is_file_reviewed(&path));
    }

    #[test]
    fn should_reset_reviewed_when_hash_changes() {
        let mut session = test_session();
        let path = PathBuf::from("changed.rs");
        session.add_file(path.clone(), FileStatus::Modified, 100);
        session.get_file_mut(&path).unwrap().reviewed = true;

        let invalidated = session.add_file(path.clone(), FileStatus::Modified, 200);
        assert!(invalidated);
        assert!(!session.is_file_reviewed(&path));
    }

    #[test]
    fn should_not_report_invalidated_for_unreviewed_file_with_changed_hash() {
        let mut session = test_session();
        let path = PathBuf::from("pending.rs");
        session.add_file(path.clone(), FileStatus::Modified, 100);

        let invalidated = session.add_file(path.clone(), FileStatus::Modified, 200);
        assert!(!invalidated);
        assert!(!session.is_file_reviewed(&path));
    }

    #[test]
    fn should_update_hash_even_when_not_reviewed() {
        let mut session = test_session();
        let path = PathBuf::from("evolving.rs");
        session.add_file(path.clone(), FileStatus::Modified, 100);
        session.add_file(path.clone(), FileStatus::Modified, 200);

        let file = session.files.get(&path).unwrap();
        assert_eq!(file.content_hash, Some(200));
    }

    /// Snapshot of a session JSON produced before PR 3 landed. New fields
    /// must deserialize with defaults; this guards against accidental
    /// breaking changes to the on-disk format.
    const LEGACY_SESSION_JSON: &str = r##"{
        "id": "abc-uuid",
        "version": "1.2",
        "repo_path": "/tmp/test-repo",
        "branch_name": "main",
        "base_commit": "deadbeef",
        "diff_source": "working_tree",
        "created_at": "2026-05-01T12:00:00Z",
        "updated_at": "2026-05-01T12:00:00Z",
        "review_comments": [],
        "files": {},
        "session_notes": null
    }"##;

    #[test]
    fn should_deserialize_pre_pr3_session_without_breakage() {
        // given a session JSON from before PR 3 landed
        // when
        let session: ReviewSession =
            serde_json::from_str(LEGACY_SESSION_JSON).expect("legacy session should parse");
        // then — new fields default to None / their default and identity is preserved
        assert_eq!(session.id, "abc-uuid");
        assert_eq!(session.base_commit, "deadbeef");
        assert_eq!(session.diff_source, SessionDiffSource::WorkingTree);
        assert!(session.pr_session_key.is_none());
        assert!(session.commit_range.is_none());
        // Per spec: an older session without remote_comments_visibility
        // defaults to `Unresolved` on read so PR-mode behavior stays sane.
        assert_eq!(
            session.remote_comments_visibility,
            PrCommentsVisibility::Unresolved
        );
        // Migration-audit §8: every known pre-thread-store version must
        // default `threads` to empty on load, not fail or fabricate data.
        assert!(session.threads().is_empty());
    }

    #[test]
    fn should_round_trip_remote_comments_visibility_on_session() {
        // given
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abcdef0123456789".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.remote_comments_visibility = PrCommentsVisibility::All;
        // when
        let json = serde_json::to_string(&session).unwrap();
        let restored: ReviewSession = serde_json::from_str(&json).unwrap();
        // then
        assert_eq!(
            restored.remote_comments_visibility,
            PrCommentsVisibility::All
        );
    }

    #[test]
    fn should_round_trip_commit_selection_range_on_pr_session() {
        // given a PR session with a strict-subset commit range selection
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abcdef0123456789".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        session.commit_selection_range = Some((1, 3));
        // when
        let json = serde_json::to_string(&session).unwrap();
        let restored: ReviewSession = serde_json::from_str(&json).unwrap();
        // then
        assert_eq!(restored.commit_selection_range, Some((1, 3)));
    }

    #[test]
    fn should_round_trip_comment_commit_id_on_session() {
        // given a session with a file comment and a line comment both scoped
        // to a single commit
        use crate::model::comment::LineSide;
        let mut session = test_session();
        session.add_file(PathBuf::from("src/lib.rs"), FileStatus::Modified, SOME_HASH);
        let file_comment = Comment::new(
            "file note on commit aaa".to_string(),
            CommentType::from_id("note"),
            None,
        )
        .with_commit_id("aaa111");
        let line_comment = Comment::new(
            "line note on commit bbb".to_string(),
            CommentType::from_id("issue"),
            Some(LineSide::New),
        )
        .with_commit_id("bbb222");
        let review = session.get_file_mut(&PathBuf::from("src/lib.rs")).unwrap();
        review.add_file_comment(file_comment.clone());
        review.add_line_comment(42, line_comment.clone());

        // when serialized and restored
        let json = serde_json::to_string(&session).unwrap();
        let restored: ReviewSession = serde_json::from_str(&json).unwrap();

        // then the commit_id survives the round trip
        let r = restored.files.get(&PathBuf::from("src/lib.rs")).unwrap();
        assert_eq!(r.file_comments.len(), 1);
        assert_eq!(
            r.file_comments[0].commit_id,
            Some("aaa111".to_string()),
            "file comment commit_id must round-trip"
        );
        let line = r.line_comments.get(&42).unwrap();
        assert_eq!(line.len(), 1);
        assert_eq!(
            line[0].commit_id,
            Some("bbb222".to_string()),
            "line comment commit_id must round-trip"
        );
    }

    #[test]
    fn should_default_commit_id_to_none_for_legacy_comment_json() {
        // given a comment JSON saved before commit_id existed
        let json = r#"{
            "id": "legacy-id",
            "content": "old note",
            "comment_type": "note",
            "created_at": "2024-01-01T00:00:00Z",
            "line_context": null,
            "side": null,
            "line_range": null,
            "author": "user",
            "lifecycle_state": "local_draft",
            "remote_review_id": null,
            "remote_comment_id": null
        }"#;
        // when
        let comment: Comment = serde_json::from_str(json).unwrap();
        // then
        assert_eq!(
            comment.commit_id, None,
            "comment JSON without commit_id must default to None"
        );
    }

    #[test]
    fn should_default_commit_selection_range_to_none_for_legacy_session() {
        // given a session JSON saved before commit_selection_range existed
        // when
        let session: ReviewSession =
            serde_json::from_str(LEGACY_SESSION_JSON).expect("legacy session should parse");
        // then
        assert_eq!(session.commit_selection_range, None);
    }

    #[test]
    fn should_round_trip_pr_session_via_serde() {
        // given
        let mut session = ReviewSession::new(
            PathBuf::from("forge:github.com/agavra/tuicr"),
            "abcdef0123456789".to_string(),
            Some("reviews".to_string()),
            SessionDiffSource::PullRequest,
        );
        let key = PrSessionKey::new(
            crate::forge::traits::ForgeRepository::github("github.com", "agavra", "tuicr"),
            125,
            "abcdef0123456789".to_string(),
        );
        session.pr_session_key = Some(key.clone());
        // when
        let json = serde_json::to_string(&session).unwrap();
        let restored: ReviewSession = serde_json::from_str(&json).unwrap();
        // then
        assert_eq!(restored.pr_session_key, Some(key));
        assert_eq!(restored.diff_source, SessionDiffSource::PullRequest);
    }

    #[test]
    fn should_default_reviewed_hunks_for_legacy_file_review() {
        let json = r#"{
            "path": "src/main.rs",
            "reviewed": false,
            "status": "modified",
            "file_comments": [],
            "line_comments": {},
            "content_hash": 123
        }"#;

        let review: FileReview = serde_json::from_str(json).unwrap();
        assert!(review.reviewed_hunks.is_empty());
    }

    #[test]
    fn should_roundtrip_reviewed_hunks() {
        let mut session = test_session();
        let file = test_diff_file("src/main.rs", vec![test_hunk(10, "same")]);
        let path = file.display_path().clone();
        let key = file.hunk_review_key(0).unwrap();

        session.add_diff_file(&file);
        session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(key.clone());

        let json = serde_json::to_string(&session).unwrap();
        let loaded: ReviewSession = serde_json::from_str(&json).unwrap();
        assert!(loaded.is_hunk_reviewed(&path, &key));
    }

    #[test]
    fn should_preserve_reviewed_hunk_when_only_line_numbers_shift() {
        let mut session = test_session();
        let original = test_diff_file("src/main.rs", vec![test_hunk(10, "same")]);
        let path = original.display_path().clone();
        let key = original.hunk_review_key(0).unwrap();

        session.add_diff_file(&original);
        session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(key.clone());

        let shifted = test_diff_file("src/main.rs", vec![test_hunk(30, "same")]);
        let shifted_key = shifted.hunk_review_key(0).unwrap();
        session.add_diff_file(&shifted);

        assert_eq!(key, shifted_key);
        assert!(session.is_hunk_reviewed(&path, &shifted_key));
    }

    #[test]
    fn should_use_line_aware_keys_for_repeated_identical_hunks() {
        let mut session = test_session();
        let original = test_diff_file(
            "src/main.rs",
            vec![test_hunk(10, "same"), test_hunk(20, "same")],
        );
        let path = original.display_path().clone();
        let first_key = original.hunk_review_key(0).unwrap();
        let second_key = original.hunk_review_key(1).unwrap();

        session.add_diff_file(&original);
        session
            .get_file_mut(&path)
            .unwrap()
            .toggle_hunk_reviewed(first_key.clone());

        let shifted = test_diff_file(
            "src/main.rs",
            vec![test_hunk(30, "same"), test_hunk(40, "same")],
        );
        session.add_diff_file(&shifted);

        assert_ne!(first_key, second_key);
        assert_ne!(first_key, shifted.hunk_review_key(0).unwrap());
        assert_ne!(second_key, shifted.hunk_review_key(1).unwrap());
        assert!(!session.is_hunk_reviewed(&path, &first_key));
        assert!(!session.is_hunk_reviewed(&path, &second_key));
    }

    #[test]
    fn should_not_move_reviewed_status_between_identical_hunks() {
        let mut session = test_session();
        let original = test_diff_file(
            "src/main.rs",
            vec![
                test_hunk(10, "same"),
                test_hunk(20, "same"),
                test_hunk(30, "same"),
            ],
        );
        let path = original.display_path().clone();
        let first_key = original.hunk_review_key(0).unwrap();
        let second_key = original.hunk_review_key(1).unwrap();
        let third_key = original.hunk_review_key(2).unwrap();

        session.add_diff_file(&original);
        let review = session.get_file_mut(&path).unwrap();
        review.toggle_hunk_reviewed(first_key.clone());
        review.toggle_hunk_reviewed(second_key.clone());

        let updated = test_diff_file(
            "src/main.rs",
            vec![
                test_hunk(10, "same"),
                test_hunk(20, "changed"),
                test_hunk(30, "same"),
            ],
        );
        let updated_first_key = updated.hunk_review_key(0).unwrap();
        let updated_third_key = updated.hunk_review_key(2).unwrap();
        session.add_diff_file(&updated);

        assert_eq!(first_key, updated_first_key);
        assert_eq!(third_key, updated_third_key);
        assert!(session.is_hunk_reviewed(&path, &updated_first_key));
        assert!(!session.is_hunk_reviewed(&path, &updated.hunk_review_key(1).unwrap()));
        assert!(!session.is_hunk_reviewed(&path, &updated_third_key));
    }

    #[test]
    fn should_prune_reviewed_hunks_that_no_longer_exist() {
        let mut session = test_session();
        let original = test_diff_file(
            "src/main.rs",
            vec![test_hunk(10, "kept"), test_hunk(20, "removed")],
        );
        let path = original.display_path().clone();
        let kept_key = original.hunk_review_key(0).unwrap();
        let removed_key = original.hunk_review_key(1).unwrap();

        session.add_diff_file(&original);
        let review = session.get_file_mut(&path).unwrap();
        review.toggle_hunk_reviewed(kept_key.clone());
        review.toggle_hunk_reviewed(removed_key.clone());

        let updated = test_diff_file(
            "src/main.rs",
            vec![test_hunk(10, "kept"), test_hunk(30, "new")],
        );
        session.add_diff_file(&updated);

        assert!(session.is_hunk_reviewed(&path, &kept_key));
        assert!(!session.is_hunk_reviewed(&path, &removed_key));
    }

    #[test]
    fn should_preserve_reviewed_hunks_for_transient_diff_views() {
        let mut session = test_session();
        let full = test_diff_file(
            "src/main.rs",
            vec![test_hunk(10, "first"), test_hunk(20, "second")],
        );
        let path = full.display_path().clone();
        let first_key = full.hunk_review_key(0).unwrap();
        let second_key = full.hunk_review_key(1).unwrap();

        session.add_diff_file(&full);
        let review = session.get_file_mut(&path).unwrap();
        review.toggle_hunk_reviewed(first_key.clone());
        review.toggle_hunk_reviewed(second_key.clone());

        let subset = test_diff_file("src/main.rs", vec![test_hunk(10, "first")]);
        session.add_diff_file_preserving_hunks(&subset);

        assert!(session.is_hunk_reviewed(&path, &first_key));
        assert!(session.is_hunk_reviewed(&path, &second_key));
    }

    #[test]
    fn should_reset_reviewed_when_legacy_session_has_no_hash() {
        let mut session = test_session();
        let path = PathBuf::from("legacy.rs");

        // Simulate a legacy session entry without content_hash.
        session.files.insert(
            path.clone(),
            FileReview {
                path: path.clone(),
                reviewed: true,
                status: FileStatus::Modified,
                file_comments: Vec::new(),
                line_comments: HashMap::new(),
                reviewed_hunks: BTreeSet::new(),
                content_hash: None,
            },
        );

        let invalidated = session.add_file(path.clone(), FileStatus::Modified, 999);
        assert!(invalidated);
        assert!(!session.is_file_reviewed(&path));
        assert_eq!(session.files.get(&path).unwrap().content_hash, Some(999));
    }

    mod thread_tests {
        use super::*;
        use crate::model::comment::LineRange;
        use crate::model::comment::LineSide;
        use crate::model::thread::{
            Anchor, AnchorTarget, AuthorKind, ThreadAuthor, ThreadComment, ThreadStatus,
        };

        #[test]
        fn should_create_new_sessions_at_current_version_with_no_threads() {
            let session = test_session();
            assert_eq!(session.version, CURRENT_SESSION_VERSION);
            assert!(session.threads().is_empty());
        }

        #[test]
        fn should_add_and_find_thread() {
            let mut session = test_session();
            let id = session.add_thread(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "root"),
            );
            assert_eq!(session.threads().len(), 1);
            let thread = session.find_thread(&id).unwrap();
            assert_eq!(thread.thread.root().unwrap().body, "root");
        }

        #[test]
        fn should_deserialize_legacy_v1_3_json_without_threads_field() {
            // Simulates a session file persisted before `threads` existed:
            // no `threads` key in the JSON at all.
            let json = serde_json::json!({
                "id": "abc",
                "version": "1.3",
                "repo_path": "/repo",
                "base_commit": "abc123",
                "created_at": Utc::now().to_rfc3339(),
                "updated_at": Utc::now().to_rfc3339(),
                "review_comments": [],
                "files": {},
                "session_notes": null,
            });
            let session: ReviewSession = serde_json::from_value(json).unwrap();
            assert_eq!(session.version, "1.3");
            assert!(session.threads().is_empty());
        }

        #[test]
        fn should_migrate_review_level_comment_to_review_anchor_thread() {
            let mut session = test_session();
            session.version = "1.3".to_string();
            session.review_comments.push(Comment::new(
                "nice work".to_string(),
                CommentType::from_id("praise"),
                None,
            ));

            let created = session.migrate_legacy_comments_to_threads();

            assert_eq!(created, 1);
            assert_eq!(session.version, CURRENT_SESSION_VERSION);
            assert_eq!(session.threads().len(), 1);
            let thread = &session.threads()[0];
            assert_eq!(*thread.thread.anchor().target(), AnchorTarget::Review);
            assert_eq!(thread.thread.status(), ThreadStatus::Open);
            assert_eq!(thread.thread.root().unwrap().body, "nice work");
            // Legacy field is preserved, not deleted.
            assert_eq!(session.review_comments.len(), 1);
        }

        #[test]
        fn should_migrate_file_level_comment_to_file_anchor_thread() {
            let mut session = test_session();
            session.version = "1.3".to_string();
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
            session
                .get_file_mut(&path)
                .unwrap()
                .add_file_comment(Comment::new(
                    "file note".to_string(),
                    CommentType::from_id("note"),
                    None,
                ));

            session.migrate_legacy_comments_to_threads();

            let thread = &session.threads()[0];
            assert_eq!(
                *thread.thread.anchor().target(),
                AnchorTarget::File {
                    path: "src/main.rs".to_string()
                }
            );
            assert!(session.files.get(&path).unwrap().file_comments.len() == 1);
        }

        #[test]
        fn should_migrate_line_comment_preserving_side_and_line() {
            let mut session = test_session();
            session.version = "1.3".to_string();
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
            let mut comment = Comment::new(
                "line note".to_string(),
                CommentType::from_id("note"),
                Some(LineSide::Old),
            );
            comment = comment.with_author("Claude Opus");
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(42, comment);

            session.migrate_legacy_comments_to_threads();

            let thread = &session.threads()[0];
            match thread.thread.anchor().target() {
                AnchorTarget::Line { path, side, line } => {
                    assert_eq!(path, "src/main.rs");
                    assert_eq!(*side, crate::model::thread::AnchorSide::Old);
                    assert_eq!(*line, 42);
                }
                other => panic!("expected a Line anchor, got {other:?}"),
            }
            assert!(
                thread.thread.anchor().context.is_none(),
                "legacy comments never captured before/after context, so a migrated \
                 old-side anchor must have no context -- it's relocation-inert (always \
                 reports Current at its last known line) rather than fabricating a \
                 zero-window context that would falsely claim content-matched confidence"
            );
            assert_eq!(thread.thread.root().unwrap().author.kind, AuthorKind::Agent);
            assert_eq!(thread.thread.root().unwrap().author.name, "Claude Opus");
        }

        #[test]
        fn should_group_independent_legacy_comments_on_the_same_line_into_one_thread() {
            // Migration-audit follow-up: legacy `line_comments:
            // HashMap<u32, Vec<Comment>>` has no root/reply distinction --
            // any number of independent `Comment`s can accumulate on the
            // same line via repeated `add_line_comment` calls. Migrating
            // each into its own one-comment thread would fragment a single
            // anchor point into several threads, contradicting the frozen
            // `Thread` model's own framing of "a discussion anchored at one
            // place". The first comment (original insertion order) must
            // become the thread's root and every later one an ordered
            // reply, all under one thread at one anchor.
            let mut session = test_session();
            session.version = "1.3".to_string();
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);

            let first = Comment::new("first note".to_string(), CommentType::None, None)
                .with_author("alice");
            let second =
                Comment::new("second note".to_string(), CommentType::None, None).with_author("bob");
            let third = Comment::new("third note".to_string(), CommentType::None, None)
                .with_author("carol");
            let (first_id, second_id, third_id) =
                (first.id.clone(), second.id.clone(), third.id.clone());

            let file = session.get_file_mut(&path).unwrap();
            file.add_line_comment(7, first);
            file.add_line_comment(7, second);
            file.add_line_comment(7, third);

            let created = session.migrate_legacy_comments_to_threads();

            assert_eq!(
                created, 1,
                "three independent comments on the same line must migrate into exactly \
                 one thread, not three"
            );
            assert_eq!(session.threads().len(), 1);
            let thread = &session.threads()[0];
            assert_eq!(thread.thread.comments().len(), 3);

            let root = thread.thread.root().unwrap();
            assert_eq!(root.body, "first note");
            assert_eq!(
                root.id().as_str(),
                first_id,
                "root must reuse the first comment's legacy id"
            );
            assert_eq!(root.author.name, "alice");

            let replies: Vec<_> = thread.thread.replies().collect();
            assert_eq!(replies.len(), 2);
            assert_eq!(replies[0].body, "second note");
            assert_eq!(
                replies[0].id().as_str(),
                second_id,
                "first reply must reuse the second comment's legacy id, not a fresh random one"
            );
            assert_eq!(replies[0].author.name, "bob");
            assert_eq!(replies[1].body, "third note");
            assert_eq!(replies[1].id().as_str(), third_id);
            assert_eq!(replies[1].author.name, "carol");
        }

        #[test]
        fn should_keep_a_line_comment_and_an_overlapping_range_comment_as_separate_threads() {
            // A bare line comment and a range comment whose *end* lands on
            // the same line share the same `line_comments` HashMap bucket
            // key, but they are genuinely different anchors -- grouping
            // must key on the fully resolved anchor (side + line_range),
            // not merely the bucket's line number, or these would be
            // incorrectly merged into one thread.
            let mut session = test_session();
            session.version = "1.3".to_string();
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);

            let line_comment =
                Comment::new("about line 7 alone".to_string(), CommentType::None, None);
            let range_comment = Comment::new_with_range(
                "about the 5-7 range".to_string(),
                CommentType::None,
                None,
                LineRange::new(5, 7),
            );

            let file = session.get_file_mut(&path).unwrap();
            file.add_line_comment(7, line_comment);
            file.add_line_comment(7, range_comment);

            let created = session.migrate_legacy_comments_to_threads();

            assert_eq!(
                created, 2,
                "different anchors sharing one bucket must stay separate"
            );
            assert_eq!(session.threads().len(), 2);
            for thread in session.threads() {
                assert_eq!(
                    thread.thread.comments().len(),
                    1,
                    "each distinct anchor keeps its own single-comment thread"
                );
            }
        }

        #[test]
        fn should_migrate_range_comment_preserving_bounds() {
            let mut session = test_session();
            session.version = "1.3".to_string();
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
            let range = LineRange::new(10, 12);
            let comment = Comment::new_with_range(
                "range note".to_string(),
                CommentType::from_id("suggestion"),
                Some(LineSide::New),
                range,
            );
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(range.end, comment);

            session.migrate_legacy_comments_to_threads();

            let thread = &session.threads()[0];
            match thread.thread.anchor().target() {
                AnchorTarget::Range {
                    path,
                    side,
                    start,
                    end,
                } => {
                    assert_eq!(path, "src/main.rs");
                    assert_eq!(*side, crate::model::thread::AnchorSide::New);
                    assert_eq!(*start, 10);
                    assert_eq!(*end, 12);
                }
                other => panic!("expected a Range anchor, got {other:?}"),
            }
        }

        #[test]
        fn should_be_idempotent_across_repeat_migration_calls() {
            let mut session = test_session();
            session.version = "1.3".to_string();
            session
                .review_comments
                .push(Comment::new("note".to_string(), CommentType::None, None));

            let first = session.migrate_legacy_comments_to_threads();
            let second = session.migrate_legacy_comments_to_threads();

            assert_eq!(first, 1);
            assert_eq!(second, 0, "repeat migration must not duplicate threads");
            assert_eq!(session.threads().len(), 1);
        }

        #[test]
        fn should_sync_a_legacy_comment_added_to_a_brand_new_current_version_session() {
            // Blocking-issue regression test: a session created fresh
            // (already at CURRENT_SESSION_VERSION, so the old
            // version-gated migration was a permanent no-op) that then
            // receives a legacy `review add` must still have that comment
            // show up via `threads()`/`review thread list` once sync runs
            // on the next load -- not be silently invisible forever.
            let mut session = test_session();
            assert_eq!(session.version, CURRENT_SESSION_VERSION);
            assert!(session.threads().is_empty());

            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
            session.get_file_mut(&path).unwrap().add_line_comment(
                7,
                Comment::new("late note".to_string(), CommentType::None, None),
            );

            let created = session.migrate_legacy_comments_to_threads();

            assert_eq!(created, 1);
            assert_eq!(session.threads().len(), 1);
            assert_eq!(
                session.threads()[0].thread.root().unwrap().body,
                "late note"
            );
            // Sync must still be a true no-op on the very next call.
            assert_eq!(session.migrate_legacy_comments_to_threads(), 0);
            assert_eq!(session.threads().len(), 1);
        }

        #[test]
        fn should_sync_a_new_comment_added_to_an_already_migrated_line_as_a_reply() {
            // The already-migrated-session variant of the same blocking
            // issue: a session that was already synced once (so it has an
            // existing thread for a line) later gets one more legacy
            // comment appended to that same line via plain `review add`.
            // The new comment must join the *existing* thread as an
            // ordered reply, not spawn a duplicate thread and not vanish.
            let mut session = test_session();
            session.version = "1.3".to_string();
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
            let first = Comment::new("first note".to_string(), CommentType::None, None);
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(7, first);

            let first_sync = session.migrate_legacy_comments_to_threads();
            assert_eq!(first_sync, 1);
            assert_eq!(session.threads().len(), 1);
            assert_eq!(session.threads()[0].thread.comments().len(), 1);

            // Simulates `review add` on the already-migrated (now
            // CURRENT_SESSION_VERSION) session: only the legacy field is
            // touched.
            let second = Comment::new("added later".to_string(), CommentType::None, None);
            let second_id = second.id.clone();
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(7, second);

            let second_sync = session.migrate_legacy_comments_to_threads();

            assert_eq!(
                second_sync, 0,
                "no *new* thread should be created; the new comment joins the existing one"
            );
            assert_eq!(
                session.threads().len(),
                1,
                "must not create a duplicate thread for the same anchor"
            );
            let thread = &session.threads()[0];
            assert_eq!(thread.thread.comments().len(), 2);
            let replies: Vec<_> = thread.thread.replies().collect();
            assert_eq!(replies.len(), 1);
            assert_eq!(replies[0].body, "added later");
            assert_eq!(replies[0].id().as_str(), second_id);
        }

        #[test]
        fn should_be_idempotent_across_repeated_load_and_save_with_no_new_comments() {
            // Simulates repeated load/save cycles (as `load_session` does
            // on every call) with no new legacy comments in between: sync
            // must be a true no-op every time, never re-appending already-
            // mirrored comments as duplicate replies and never creating
            // duplicate threads.
            let mut session = test_session();
            session.version = "1.3".to_string();
            session.review_comments.push(Comment::new(
                "review note".to_string(),
                CommentType::None,
                None,
            ));
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
            session
                .get_file_mut(&path)
                .unwrap()
                .add_file_comment(Comment::new(
                    "file note".to_string(),
                    CommentType::None,
                    None,
                ));
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(3, Comment::new("a".to_string(), CommentType::None, None));
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(3, Comment::new("b".to_string(), CommentType::None, None));

            let first = session.migrate_legacy_comments_to_threads();
            assert_eq!(first, 3, "review + file + one grouped line thread");
            let after_first: Vec<_> = session.threads().to_vec();

            for _ in 0..5 {
                let created = session.migrate_legacy_comments_to_threads();
                assert_eq!(
                    created, 0,
                    "repeated sync with no new comments must be a no-op"
                );
            }

            assert_eq!(session.threads().len(), after_first.len());
            assert_eq!(
                session.threads(),
                after_first.as_slice(),
                "repeated sync must not mutate already-synced threads (no duplicate replies)"
            );
        }

        #[test]
        fn should_preserve_same_anchor_grouping_when_syncing_incrementally_added_comments() {
            // Same-anchor grouping semantics must hold not just for a
            // one-shot migration of a fully-formed legacy session (already
            // covered by `should_group_independent_legacy_comments_on_the_
            // same_line_into_one_thread`), but also when comments are
            // added incrementally across multiple sync calls -- the kind
            // of sequence a real `review add` workflow produces.
            let mut session = test_session();
            session.version = "1.3".to_string();
            let path = PathBuf::from("src/main.rs");
            session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);

            let first = Comment::new("one".to_string(), CommentType::None, None);
            let first_id = first.id.clone();
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(10, first);
            session.migrate_legacy_comments_to_threads();

            // Two more independent comments land on the same line across
            // two separate later `review add` + sync cycles.
            let second = Comment::new("two".to_string(), CommentType::None, None);
            let second_id = second.id.clone();
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(10, second);
            session.migrate_legacy_comments_to_threads();

            let third = Comment::new("three".to_string(), CommentType::None, None);
            let third_id = third.id.clone();
            session
                .get_file_mut(&path)
                .unwrap()
                .add_line_comment(10, third);
            session.migrate_legacy_comments_to_threads();

            assert_eq!(
                session.threads().len(),
                1,
                "all three same-line comments must stay grouped under one thread"
            );
            let thread = &session.threads()[0];
            assert_eq!(thread.thread.comments().len(), 3);
            assert_eq!(thread.thread.root().unwrap().id().as_str(), first_id);
            let replies: Vec<_> = thread.thread.replies().collect();
            assert_eq!(replies[0].id().as_str(), second_id);
            assert_eq!(replies[1].id().as_str(), third_id);
        }

        #[test]
        fn should_derive_identical_thread_and_comment_ids_across_independent_migrations() {
            // Two independently-built sessions from byte-identical legacy
            // input (same comment id, content, anchor) must migrate to
            // byte-identical thread/comment identities -- not just the
            // same *count* of threads. This is what makes it safe to run
            // migration in-memory on every load of the same underlying
            // legacy file across separate processes without ever
            // persisting two different identities for the same comment.
            // Fixed once so both builds migrate the exact same legacy
            // comment id -- simulating two separate processes loading the
            // same on-disk legacy session file.
            let comment = Comment::new("note".to_string(), CommentType::None, None);
            let build = |comment: &Comment| {
                let mut session = test_session();
                session.version = "1.3".to_string();
                session.review_comments.push(comment.clone());
                session.migrate_legacy_comments_to_threads();
                session.threads().to_vec()
            };

            let first = build(&comment);
            let second = build(&comment);

            assert_eq!(first.len(), 1);
            assert_eq!(second.len(), 1);
            assert_eq!(
                first[0].id(),
                second[0].id(),
                "ThreadId must be deterministic, not randomly minted per migration run"
            );
            assert_eq!(
                first[0].thread.root().unwrap().id(),
                second[0].thread.root().unwrap().id(),
                "root CommentId must be deterministic, not randomly minted per migration run"
            );
        }

        #[test]
        fn should_migrate_deterministically_across_multiple_files_and_lines() {
            let build = || {
                let mut session = test_session();
                session.version = "1.3".to_string();
                for (path, line) in [("b.rs", 5u32), ("a.rs", 2u32)] {
                    let path = PathBuf::from(path);
                    session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
                    session.get_file_mut(&path).unwrap().add_line_comment(
                        line,
                        Comment::new(
                            format!("note on {}", path.display()),
                            CommentType::None,
                            None,
                        ),
                    );
                }
                session.migrate_legacy_comments_to_threads();
                session
                    .threads()
                    .iter()
                    .map(|t| t.thread.anchor().target().path().unwrap().to_string())
                    .collect::<Vec<_>>()
            };

            // Same structural mapping every time: a.rs before b.rs (path order).
            assert_eq!(build(), vec!["a.rs".to_string(), "b.rs".to_string()]);
            assert_eq!(build(), vec!["a.rs".to_string(), "b.rs".to_string()]);
        }

        #[test]
        fn should_migrate_lines_within_one_file_in_stable_sorted_order_despite_hashmap_iteration() {
            // `line_comments` is a `HashMap<u32, Vec<Comment>>`; its
            // iteration order is randomized per-instance and can differ
            // across otherwise-identical builds within the same process,
            // not just across runs. This seeds many lines in scrambled
            // insertion order and asserts the migrated thread order is
            // always sorted by line number, proving `migrate_legacy_
            // comments_to_threads`'s explicit `lines.sort()` (not
            // HashMap iteration) is what determines output order.
            let build = || {
                let mut session = test_session();
                session.version = "1.3".to_string();
                let path = PathBuf::from("scrambled.rs");
                session.add_file(path.clone(), FileStatus::Modified, SOME_HASH);
                // Deliberately out-of-order insertion.
                for line in [40u32, 10, 30, 20, 50, 1, 25] {
                    session.get_file_mut(&path).unwrap().add_line_comment(
                        line,
                        Comment::new(format!("note on line {line}"), CommentType::None, None),
                    );
                }
                session.migrate_legacy_comments_to_threads();
                session
                    .threads()
                    .iter()
                    .map(|t| match t.thread.anchor().target() {
                        crate::model::thread::AnchorTarget::Line { line, .. } => *line,
                        other => unreachable!(
                            "all threads in this test are line-anchored, got {other:?}"
                        ),
                    })
                    .collect::<Vec<_>>()
            };

            let expected = vec![1u32, 10, 20, 25, 30, 40, 50];
            for _ in 0..5 {
                assert_eq!(build(), expected);
            }
        }

        #[test]
        fn should_roundtrip_threads_through_session_json() {
            let mut session = test_session();
            session.add_thread(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "root"),
            );

            let json = serde_json::to_string(&session).unwrap();
            let restored: ReviewSession = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.threads().len(), 1);
            assert_eq!(restored.threads()[0].thread.root().unwrap().body, "root");
        }

        fn remote_thread(
            id: &str,
            path: &str,
            line: Option<u32>,
            is_resolved: bool,
            is_outdated: bool,
            comment_bodies: &[(&str, &str)],
        ) -> crate::forge::remote_comments::RemoteReviewThread {
            use crate::forge::remote_comments::{
                RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
            };
            RemoteReviewThread {
                id: id.to_string(),
                path: path.to_string(),
                line,
                side: RemoteCommentSide::Right,
                is_resolved,
                is_outdated,
                comments: comment_bodies
                    .iter()
                    .map(|(author, body)| RemoteReviewComment {
                        id: format!("{id}-{author}"),
                        author: Some(author.to_string()),
                        body: body.to_string(),
                        // Fixed rather than `None` so re-importing the same
                        // logical remote payload twice in a test is
                        // deterministic: `None` falls back to `Utc::now()`
                        // in `thread_store::remote_thread_comment`, which
                        // would make every reimport mint a fresh timestamp
                        // even when nothing else in the payload changed.
                        created_at: Some(chrono::DateTime::UNIX_EPOCH),
                        in_reply_to: None,
                        url: format!("https://example.invalid/{id}"),
                    })
                    .collect(),
            }
        }

        #[test]
        fn should_import_remote_thread_preserving_root_reply_order_ids_and_authors() {
            let mut session = test_session();
            let remote = remote_thread(
                "R_1",
                "src/lib.rs",
                Some(42),
                false,
                false,
                &[("alice", "why here?"), ("bob", "good question")],
            );

            let created =
                session.import_remote_review_threads("github", std::slice::from_ref(&remote));
            assert_eq!(created, 1);
            assert_eq!(session.threads().len(), 1);

            let thread = &session.threads()[0].thread;
            assert_eq!(thread.status(), ThreadStatus::Open);
            assert_eq!(thread.comments().len(), 2);
            assert_eq!(thread.root().unwrap().body, "why here?");
            assert_eq!(thread.root().unwrap().author.name, "alice");
            assert_eq!(thread.root().unwrap().author.kind, AuthorKind::Remote);
            assert_eq!(thread.root().unwrap().id().as_str(), "R_1-alice");
            let reply = thread.replies().next().unwrap();
            assert_eq!(reply.body, "good question");
            assert_eq!(reply.author.name, "bob");
            assert_eq!(reply.id().as_str(), "R_1-bob");

            match thread.anchor().target() {
                AnchorTarget::Line { path, line, .. } => {
                    assert_eq!(path, "src/lib.rs");
                    assert_eq!(*line, 42);
                }
                other => panic!("expected a line anchor, got {other:?}"),
            }

            assert!(
                session.threads()[0].has_provider_id("github", "R_1"),
                "provider mapping must record the remote thread id"
            );
        }

        #[test]
        fn should_not_duplicate_thread_when_reimporting_the_same_remote_thread_twice() {
            let mut session = test_session();
            let remote = remote_thread("R_2", "a.rs", Some(5), false, false, &[("alice", "note")]);

            let first =
                session.import_remote_review_threads("github", std::slice::from_ref(&remote));
            let second =
                session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            assert_eq!(first, 1, "first import creates exactly one thread");
            assert_eq!(second, 0, "re-import must not create a new thread");
            assert_eq!(
                session.threads().len(),
                1,
                "re-fetching twice must not duplicate the thread"
            );
        }

        #[test]
        fn should_reflect_updated_remote_resolution_state_on_reimport() {
            let mut session = test_session();
            let open_remote =
                remote_thread("R_3", "a.rs", Some(5), false, false, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&open_remote));
            assert_eq!(session.threads()[0].thread.status(), ThreadStatus::Open);

            let resolved_remote =
                remote_thread("R_3", "a.rs", Some(5), true, false, &[("alice", "note")]);
            let created = session
                .import_remote_review_threads("github", std::slice::from_ref(&resolved_remote));

            assert_eq!(
                created, 0,
                "updating an existing import is not a new thread"
            );
            assert_eq!(session.threads().len(), 1);
            assert_eq!(session.threads()[0].thread.status(), ThreadStatus::Resolved);
        }

        #[test]
        fn should_import_fully_outdated_remote_thread_as_file_anchor() {
            let mut session = test_session();
            let remote =
                remote_thread("R_4", "a.rs", None, false, true, &[("alice", "stale note")]);

            session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            match session.threads()[0].thread.anchor().target() {
                AnchorTarget::File { path } => assert_eq!(path, "a.rs"),
                other => {
                    panic!("expected a file anchor for a lineless remote thread, got {other:?}")
                }
            }
        }

        #[test]
        fn should_keep_remote_threads_from_different_providers_distinct() {
            let mut session = test_session();
            let remote = remote_thread("R_5", "a.rs", Some(1), false, false, &[("alice", "note")]);

            session.import_remote_review_threads("github", std::slice::from_ref(&remote));
            session.import_remote_review_threads("gitlab", std::slice::from_ref(&remote));

            assert_eq!(
                session.threads().len(),
                2,
                "the same remote id under different providers must not collapse into one thread"
            );
        }

        // Audit findings 5/8: re-importing an already-imported remote thread
        // must *merge* into the existing entry, not silently overwrite it —
        // preserving local-only replies, never regressing a local
        // resolve/dismiss/stale/ambiguous state, and never clobbering a
        // locally-relocated anchor.

        #[test]
        fn should_preserve_a_locally_added_reply_when_remote_thread_is_reimported() {
            let mut session = test_session();
            let remote = remote_thread("R_6", "a.rs", Some(5), false, false, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            let thread_id = session.threads()[0].id().clone();
            session
                .find_thread_mut(&thread_id)
                .unwrap()
                .thread
                .reply(ThreadComment::new(
                    ThreadAuthor::human("carol"),
                    "local reply",
                ));

            // Re-fetch the exact same remote state (no provider-side change).
            let created =
                session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            assert_eq!(created, 0);
            assert_eq!(session.threads().len(), 1);
            let thread = &session.threads()[0].thread;
            assert_eq!(
                thread.comments().len(),
                2,
                "the locally-added reply must survive the re-import"
            );
            let local_reply = thread.replies().next().unwrap();
            assert_eq!(local_reply.body, "local reply");
            assert_eq!(local_reply.author.name, "carol");
            assert_eq!(local_reply.author.kind, AuthorKind::Human);
        }

        #[test]
        fn should_pick_up_a_new_remote_reply_alongside_a_preserved_local_reply_on_reimport() {
            let mut session = test_session();
            let remote = remote_thread("R_7", "a.rs", Some(5), false, false, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            let thread_id = session.threads()[0].id().clone();
            session
                .find_thread_mut(&thread_id)
                .unwrap()
                .thread
                .reply(ThreadComment::new(
                    ThreadAuthor::human("carol"),
                    "local reply",
                ));

            // The provider now shows a second remote reply that wasn't there
            // on first fetch.
            let updated_remote = remote_thread(
                "R_7",
                "a.rs",
                Some(5),
                false,
                false,
                &[("alice", "note"), ("bob", "remote follow-up")],
            );
            session.import_remote_review_threads("github", std::slice::from_ref(&updated_remote));

            let thread = &session.threads()[0].thread;
            assert_eq!(
                thread.comments().len(),
                3,
                "root + remote reply + local reply"
            );
            let bodies: Vec<&str> = thread
                .replies()
                .map(|comment| comment.body.as_str())
                .collect();
            assert_eq!(
                bodies,
                vec!["remote follow-up", "local reply"],
                "new remote reply must land before the preserved local reply"
            );
        }

        #[test]
        fn should_not_regress_a_local_resolve_when_remote_reimport_still_reports_open() {
            let mut session = test_session();
            let remote = remote_thread("R_8", "a.rs", Some(5), false, false, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            let thread_id = session.threads()[0].id().clone();
            session
                .find_thread_mut(&thread_id)
                .unwrap()
                .thread
                .resolve();

            // A stale/lagging re-fetch still reports the thread as open.
            let created =
                session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            assert_eq!(created, 0);
            assert_eq!(
                session.threads()[0].thread.status(),
                ThreadStatus::Resolved,
                "a local resolve must survive a stale remote re-fetch reporting still-open"
            );
        }

        #[test]
        fn should_not_regress_a_local_dismiss_when_remote_reimport_reports_resolved() {
            let mut session = test_session();
            let remote = remote_thread("R_9", "a.rs", Some(5), false, false, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            let thread_id = session.threads()[0].id().clone();
            session
                .find_thread_mut(&thread_id)
                .unwrap()
                .thread
                .dismiss();

            let resolved_remote =
                remote_thread("R_9", "a.rs", Some(5), true, false, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&resolved_remote));

            assert_eq!(
                session.threads()[0].thread.status(),
                ThreadStatus::Dismissed,
                "a local dismiss is terminal and must never be overwritten by a remote resolve"
            );
        }

        #[test]
        fn should_not_clobber_a_locally_relocated_stale_anchor_on_remote_reimport() {
            // `thread_from_remote` builds a context-less anchor
            // (`Anchor::line`), so `refresh_anchor` is currently always a
            // no-op for it (anchors with no captured context are left
            // untouched — see `Anchor::relocate_with_remap`'s doc comment).
            // Simulate a thread that *did* go Stale (e.g. via a future
            // context-capturing enhancement, or any other path that ends up
            // sharing this thread's deterministic remote-derived ID) by
            // constructing it directly with a context-bearing anchor, the
            // same way `dryrun.rs`'s own Stale-anchor tests do. This still
            // exercises exactly the real merge contract under test: once a
            // thread with this ID is Stale, reimporting the same remote
            // thread must never clobber that anchor/status.
            let mut session = test_session();
            let thread_id = crate::model::thread_store::remote_thread_id_for("github", "R_10");
            let anchor = Anchor::line_with_context(
                "a.rs",
                crate::model::thread::AnchorSide::New,
                5,
                crate::model::AnchorContext {
                    before: vec![],
                    selected: vec!["original content".to_string()],
                    after: vec![],
                },
            )
            .unwrap();
            let root = ThreadComment::new(ThreadAuthor::remote("alice", "alice"), "note");
            let mut thread: crate::model::thread::Thread =
                serde_json::from_value(serde_json::json!({
                    "id": thread_id,
                    "status": "open",
                    "anchor": anchor,
                    "comments": [root],
                }))
                .unwrap();
            thread.refresh_anchor(&["completely", "different", "file", "contents"]);
            assert_eq!(
                thread.status(),
                ThreadStatus::Stale,
                "setup: the anchor refresh above must have produced a Stale thread"
            );
            let mut persisted = PersistedThread::new(thread);
            persisted.upsert_provider_mapping(
                "github",
                serde_json::json!({"id": "R_10", "path": "a.rs", "line": 5, "is_outdated": false}),
            );
            session.threads.push(persisted);

            // The provider now reports a different line for the same thread
            // (e.g. its own line-tracking caught up) — this must not reset
            // our locally-computed Stale anchor/status back to "current".
            let moved_remote =
                remote_thread("R_10", "a.rs", Some(9), false, false, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&moved_remote));

            let thread = &session.threads()[0].thread;
            assert_eq!(
                thread.status(),
                ThreadStatus::Stale,
                "local Stale status must survive a remote reimport"
            );
            match thread.anchor().target() {
                AnchorTarget::Line { line, .. } => assert_eq!(
                    *line, 5,
                    "local anchor line must not be silently overwritten by the remote's line"
                ),
                other => panic!("expected a line anchor, got {other:?}"),
            }
        }

        #[test]
        fn should_still_capture_native_resolved_flag_in_provider_mapping_even_when_status_is_preserved()
         {
            // Finding 8: reconcile provider resolved/outdated with local
            // stale/ambiguous *without* lossy overwrite — the provider's own
            // flag must still be recorded even though `status` itself isn't
            // force-transitioned. Uses the same directly-constructed Stale
            // thread technique as the test above (see its comment for why).
            let mut session = test_session();
            let thread_id = crate::model::thread_store::remote_thread_id_for("github", "R_11");
            let anchor = Anchor::line_with_context(
                "a.rs",
                crate::model::thread::AnchorSide::New,
                5,
                crate::model::AnchorContext {
                    before: vec![],
                    selected: vec!["original content".to_string()],
                    after: vec![],
                },
            )
            .unwrap();
            let root = ThreadComment::new(ThreadAuthor::remote("alice", "alice"), "note");
            let mut thread: crate::model::thread::Thread =
                serde_json::from_value(serde_json::json!({
                    "id": thread_id,
                    "status": "open",
                    "anchor": anchor,
                    "comments": [root],
                }))
                .unwrap();
            thread.refresh_anchor(&["completely", "different", "file", "contents"]);
            let mut persisted = PersistedThread::new(thread);
            persisted.upsert_provider_mapping(
                "github",
                serde_json::json!({"id": "R_11", "path": "a.rs", "line": 5, "is_outdated": false}),
            );
            session.threads.push(persisted);

            let resolved_remote =
                remote_thread("R_11", "a.rs", Some(5), true, true, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&resolved_remote));

            assert_eq!(session.threads()[0].thread.status(), ThreadStatus::Stale);
            let mapping = session.threads()[0].provider_mapping("github").unwrap();
            assert_eq!(mapping["is_resolved"], serde_json::json!(true));
            assert_eq!(mapping["is_outdated"], serde_json::json!(true));
        }

        #[test]
        fn should_be_idempotent_when_reimporting_an_unchanged_remote_thread_after_local_reply() {
            // Direct finding-5 regression: reimporting the *exact same*
            // remote payload a second time (no provider-side change) is a
            // strict no-op even when a local reply was added in between.
            let mut session = test_session();
            let remote = remote_thread("R_12", "a.rs", Some(5), false, false, &[("alice", "note")]);
            session.import_remote_review_threads("github", std::slice::from_ref(&remote));

            let thread_id = session.threads()[0].id().clone();
            session
                .find_thread_mut(&thread_id)
                .unwrap()
                .thread
                .reply(ThreadComment::new(
                    ThreadAuthor::human("carol"),
                    "local reply",
                ));

            let before = session.threads()[0].clone();
            session.import_remote_review_threads("github", std::slice::from_ref(&remote));
            session.import_remote_review_threads("github", std::slice::from_ref(&remote));
            let after = session.threads()[0].clone();

            assert_eq!(session.threads().len(), 1);
            assert_eq!(
                before, after,
                "re-importing an unchanged remote thread twice must be a byte-identical no-op"
            );
        }
    }
}
