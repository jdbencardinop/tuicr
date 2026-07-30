use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use super::comment::Comment;
use super::diff_types::{DiffFile, FileStatus};
use super::thread::{Anchor, ThreadComment, ThreadId};
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

    /// Deterministically and idempotently convert this session's legacy
    /// `review_comments`/`files[..].file_comments`/`files[..].line_comments`
    /// into durable [`PersistedThread`]s.
    ///
    /// No-op (returns `0`) once `self.version` is already
    /// [`CURRENT_SESSION_VERSION`], so calling this on every session load is
    /// safe and never duplicates threads. Legacy comment fields are never
    /// mutated or removed — this only ever *adds* threads — so existing
    /// public API/JSON consumers reading `review_comments`/`files` keep
    /// working unchanged after migration.
    ///
    /// Ordering is deterministic: review-level comments first (in their
    /// existing order), then per-file comments in file-path order, then
    /// file-level comments followed by line/range comments in ascending
    /// line order. Each legacy `Comment` becomes its own single-comment
    /// thread (the legacy format has no reply/thread concept), starting
    /// `Open`. See [`thread_store::thread_from_legacy_comment`] for the
    /// author-kind migration heuristic.
    pub fn migrate_legacy_comments_to_threads(&mut self) -> usize {
        if self.version == CURRENT_SESSION_VERSION {
            return 0;
        }

        let mut created = 0;

        for comment in &self.review_comments {
            self.threads.push(thread_store::thread_from_legacy_comment(
                Anchor::review(),
                comment,
            ));
            created += 1;
        }

        let mut paths: Vec<_> = self.files.keys().cloned().collect();
        paths.sort();
        for path in paths {
            let path_str = path.to_string_lossy().to_string();
            let review = &self.files[&path];

            for comment in review.file_comments.clone() {
                self.threads.push(thread_store::thread_from_legacy_comment(
                    Anchor::file(path_str.clone()),
                    &comment,
                ));
                created += 1;
            }

            let mut lines: Vec<_> = review.line_comments.keys().copied().collect();
            lines.sort();
            for line in lines {
                let comments = self.files[&path].line_comments[&line].clone();
                for comment in comments {
                    let side = thread_store::anchor_side_for_legacy(comment.side);
                    let anchor = match comment.line_range {
                        Some(range) => {
                            Anchor::range(path_str.clone(), side, range.start, range.end)
                                .expect("LineRange is always normalized start <= end")
                        }
                        None => Anchor::line(path_str.clone(), side, line),
                    };
                    self.threads
                        .push(thread_store::thread_from_legacy_comment(anchor, &comment));
                    created += 1;
                }
            }
        }

        self.version = CURRENT_SESSION_VERSION.to_string();
        created
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
    }
}
