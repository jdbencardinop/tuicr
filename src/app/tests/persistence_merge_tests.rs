use crate::app::*;
use crate::model::thread::{Anchor, Thread, ThreadAuthor, ThreadComment};

const SOME_HASH: u64 = 0xabc;

fn test_session() -> ReviewSession {
    let mut session = ReviewSession::new(
        PathBuf::from("/repo"),
        "abc1234".to_string(),
        Some("main".to_string()),
        SessionDiffSource::WorkingTree,
    );
    session.add_file(
        PathBuf::from("src/main.rs"),
        FileStatus::Modified,
        SOME_HASH,
    );
    session
}

fn comment(id: &str, content: &str) -> Comment {
    let mut comment = Comment::new(content.to_string(), CommentType::from_id("note"), None);
    comment.id = id.to_string();
    comment
}

fn push_file_comment(session: &mut ReviewSession, id: &str, content: &str) {
    session
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments
        .push(comment(id, content));
}

fn file_comment_ids(session: &ReviewSession) -> Vec<String> {
    session
        .files
        .get(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments
        .iter()
        .map(|comment| comment.id.clone())
        .collect()
}

fn thread_at(session: &ReviewSession, id: &crate::model::thread::ThreadId) -> PersistedThread {
    session
        .threads
        .iter()
        .find(|thread| thread.id() == id)
        .cloned()
        .expect("thread present")
}

fn open_review_thread(author: &str, body: &str) -> PersistedThread {
    let root = ThreadComment::new(ThreadAuthor::human(author), body);
    PersistedThread::new(Thread::open(Anchor::review(), root))
}

#[test]
fn should_merge_external_comment_without_losing_local_comment() {
    let base = test_session();
    let mut current = base.clone();
    let mut latest = base.clone();

    push_file_comment(&mut current, "local", "from tui");
    push_file_comment(&mut latest, "external", "from cli");

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 1);
    assert_eq!(file_comment_ids(&current), vec!["local", "external"]);
}

#[test]
fn should_not_resurrect_locally_deleted_comment_when_disk_is_unchanged() {
    let mut base = test_session();
    push_file_comment(&mut base, "deleted", "old");
    let mut current = base.clone();
    current
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments
        .clear();
    let latest = base.clone();

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 0);
    assert!(file_comment_ids(&current).is_empty());
}

#[test]
fn should_apply_external_edit_when_comment_is_unchanged_locally() {
    let mut base = test_session();
    push_file_comment(&mut base, "same", "old");
    let mut current = base.clone();
    let mut latest = base.clone();
    latest
        .get_file_mut(&PathBuf::from("src/main.rs"))
        .unwrap()
        .file_comments[0]
        .content = "new".to_string();

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 1);
    assert_eq!(
        current
            .files
            .get(&PathBuf::from("src/main.rs"))
            .unwrap()
            .file_comments[0]
            .content,
        "new"
    );
}

// Audit finding 1: `merge_external_session_changes`/poll must three-way
// merge `threads` too, not just legacy comments/reviewed marks, so an
// external reply/resolve/import survives a TUI save and becomes visible
// after a poll instead of being silently clobbered by the TUI's stale
// in-memory `current.threads`.

#[test]
fn should_add_a_brand_new_external_thread_without_losing_local_thread() {
    let base = test_session();
    let mut current = base.clone();
    let mut latest = base.clone();

    let local_thread = open_review_thread("alice", "local thread");
    let local_id = local_thread.id().clone();
    current.threads.push(local_thread);

    let external_thread = open_review_thread("bob", "external thread");
    let external_id = external_thread.id().clone();
    latest.threads.push(external_thread);

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 1);
    assert_eq!(current.threads.len(), 2);
    assert!(current.threads.iter().any(|t| t.id() == &local_id));
    assert!(current.threads.iter().any(|t| t.id() == &external_id));
}

#[test]
fn should_survive_external_reply_added_between_snapshot_and_tui_save() {
    // Simulate: TUI opens a thread (base == current, untouched locally),
    // then something external (e.g. a `ReviewStore`-mediated CLI reply, or
    // a background PR-thread refresh) appends a reply to the on-disk
    // session before the TUI's own next save. The external reply must
    // survive the merge rather than being overwritten by the TUI's stale
    // in-memory copy.
    let mut base = test_session();
    let thread = open_review_thread("alice", "root");
    let thread_id = thread.id().clone();
    base.threads.push(thread);

    let current = base.clone();
    let mut latest = base.clone();
    latest
        .threads
        .iter_mut()
        .find(|t| t.id() == &thread_id)
        .unwrap()
        .thread
        .reply(ThreadComment::new(
            ThreadAuthor::human("bob"),
            "external reply",
        ));

    let mut merged = current;
    let changed = App::merge_external_session_changes(&mut merged, &base, &latest);

    assert_eq!(changed, 1);
    let merged_thread = thread_at(&merged, &thread_id);
    assert_eq!(merged_thread.thread.replies().count(), 1);
    assert_eq!(
        merged_thread.thread.replies().next().unwrap().body,
        "external reply"
    );
}

#[test]
fn should_survive_external_resolve_between_snapshot_and_tui_save() {
    let mut base = test_session();
    let thread = open_review_thread("alice", "root");
    let thread_id = thread.id().clone();
    base.threads.push(thread);

    let current = base.clone();
    let mut latest = base.clone();
    latest
        .threads
        .iter_mut()
        .find(|t| t.id() == &thread_id)
        .unwrap()
        .thread
        .resolve();

    let mut merged = current;
    let changed = App::merge_external_session_changes(&mut merged, &base, &latest);

    assert_eq!(changed, 1);
    assert_eq!(
        thread_at(&merged, &thread_id).thread.status(),
        crate::model::thread::ThreadStatus::Resolved
    );
}

#[test]
fn should_prefer_local_thread_edit_over_concurrent_external_edit_of_same_thread() {
    // Mirrors `should_apply_external_edit_when_comment_is_unchanged_locally`'s
    // sibling rule for comments: if the TUI itself already mutated a thread
    // (e.g. the user resolved it locally) since the last snapshot, a
    // *different* concurrent external edit to that same thread must not
    // clobber the local change.
    let mut base = test_session();
    let thread = open_review_thread("alice", "root");
    let thread_id = thread.id().clone();
    base.threads.push(thread);

    let mut current = base.clone();
    current
        .threads
        .iter_mut()
        .find(|t| t.id() == &thread_id)
        .unwrap()
        .thread
        .resolve();

    let mut latest = base.clone();
    latest
        .threads
        .iter_mut()
        .find(|t| t.id() == &thread_id)
        .unwrap()
        .thread
        .reply(ThreadComment::new(
            ThreadAuthor::human("bob"),
            "external reply",
        ));

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 0);
    let merged_thread = thread_at(&current, &thread_id);
    assert_eq!(
        merged_thread.thread.status(),
        crate::model::thread::ThreadStatus::Resolved
    );
    assert_eq!(merged_thread.thread.replies().count(), 0);
}

#[test]
fn should_not_resurrect_a_locally_deleted_thread_when_disk_is_unchanged() {
    let mut base = test_session();
    let thread = open_review_thread("alice", "root");
    let thread_id = thread.id().clone();
    base.threads.push(thread);

    let mut current = base.clone();
    current.threads.retain(|t| t.id() != &thread_id);
    let latest = base.clone();

    let changed = App::merge_external_session_changes(&mut current, &base, &latest);

    assert_eq!(changed, 0);
    assert!(current.threads.is_empty());
}

#[test]
fn should_merge_publication_mapping_with_concurrent_local_reply() {
    let mut base = test_session();
    let thread = open_review_thread("alice", "root");
    let thread_id = thread.id().clone();
    base.threads.push(thread);
    let mut current = base.clone();
    let mut latest = base.clone();

    current
        .find_thread_mut(&thread_id)
        .unwrap()
        .thread
        .reply(ThreadComment::new(
            ThreadAuthor::human("alice"),
            "local reply",
        ));
    latest
        .find_thread_mut(&thread_id)
        .unwrap()
        .upsert_provider_mapping("gitlab", serde_json::json!({"id": "discussion-1"}));

    App::merge_external_session_changes(&mut current, &base, &latest);

    let merged = current.find_thread(&thread_id).unwrap();
    assert_eq!(merged.thread.replies().count(), 1);
    assert_eq!(
        merged
            .provider_mapping("gitlab")
            .and_then(|mapping| mapping.get("id"))
            .and_then(|id| id.as_str()),
        Some("discussion-1")
    );
}

#[test]
fn should_union_concurrent_publication_reply_ledgers() {
    let mut base = test_session();
    let thread = open_review_thread("alice", "root");
    let thread_id = thread.id().clone();
    base.threads.push(thread);
    let mut current = base.clone();
    let mut latest = base.clone();

    current
        .find_thread_mut(&thread_id)
        .unwrap()
        .record_published_reply("gitlab", "local-a", "note-a");
    latest
        .find_thread_mut(&thread_id)
        .unwrap()
        .record_published_reply("gitlab", "local-b", "note-b");

    App::merge_external_session_changes(&mut current, &base, &latest);

    let merged = current.find_thread(&thread_id).unwrap();
    assert_eq!(
        merged.published_reply_id("gitlab", "local-a"),
        Some("note-a")
    );
    assert_eq!(
        merged.published_reply_id("gitlab", "local-b"),
        Some("note-b")
    );
}

#[test]
fn should_reload_persisted_session_and_surface_external_thread_activity() {
    // End-to-end (not just the pure merge function): a poll/reload must
    // both merge the thread into `self.session` *and* report a non-zero
    // changed count so the caller (main loop) schedules a redraw, even
    // when the only external change is thread activity (no legacy
    // comments involved at all).
    struct NoopVcs {
        info: VcsInfo,
    }
    impl VcsBackend for NoopVcs {
        fn info(&self) -> &VcsInfo {
            &self.info
        }
        fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
            Err(TuicrError::NoChanges)
        }
        fn fetch_context_lines(
            &self,
            _file_path: &Path,
            _file_status: crate::model::FileStatus,
            _ref_commit: Option<&str>,
            _start_line: u32,
            _end_line: u32,
        ) -> Result<Vec<DiffLine>> {
            Ok(Vec::new())
        }
        fn file_line_count(
            &self,
            _file_path: &Path,
            _file_status: crate::model::FileStatus,
            _ref_commit: Option<&str>,
        ) -> Result<u32> {
            Ok(0)
        }
    }

    let tmp = tempfile::tempdir().unwrap();
    let vcs_info = VcsInfo {
        root_path: tmp.path().to_path_buf(),
        head_commit: "deadbeef".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: crate::vcs::traits::VcsType::Git,
    };
    let mut session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    let thread = open_review_thread("alice", "root");
    let thread_id = thread.id().clone();
    session.threads.push(thread);

    let path = crate::persistence::storage::save_session(&session).unwrap();

    let mut app = App::build(
        Box::new(NoopVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        Vec::new(),
        session.clone(),
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app");
    app.session_path = Some(path.clone());
    app.session_file_state = SessionFileState::from_path(&path).ok();
    app.persisted_session_snapshot = session.clone();

    // Simulate an external writer (e.g. a `ReviewStore` reply, or a
    // background PR-thread refresh) appending a reply and re-saving.
    let mut external = session.clone();
    external
        .threads
        .iter_mut()
        .find(|t| t.id() == &thread_id)
        .unwrap()
        .thread
        .reply(ThreadComment::new(
            ThreadAuthor::human("bob"),
            "external reply",
        ));
    crate::persistence::storage::save_session(&external).unwrap();

    let changed = app
        .reload_persisted_session_if_changed(true)
        .expect("reload succeeds");

    assert_eq!(
        changed, 1,
        "thread-only external change must be reported as a change"
    );
    let reloaded_thread = thread_at(&app.session, &thread_id);
    assert_eq!(reloaded_thread.thread.replies().count(), 1);
}
