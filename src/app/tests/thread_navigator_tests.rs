//! Regression tests for audit findings 2/3: durable-thread content that
//! has no legacy `Comment` mirror of its own (a TUI-authored reply via
//! `Thread::reply`) must still get a real `AnnotatedLine` entry — not
//! just a raw rendered row — so the cursor, navigator, and reply/resolve/
//! dismiss keybindings can reach it by `ThreadId` directly, without going
//! through a positional legacy `CommentLocation` lookup, and without
//! desyncing `line_annotations` against the actually-rendered rows.

use crate::app::*;
use crate::model::{DiffFile, DiffHunk, DiffLine, FileStatus, LineOrigin};
use crate::review_store::{AddCommentRequest, CommentTarget, add_comment_to_session};
use crate::vcs::traits::VcsType;

struct DummyVcs {
    info: VcsInfo,
}

impl VcsBackend for DummyVcs {
    fn info(&self) -> &VcsInfo {
        &self.info
    }
    fn get_working_tree_diff(&self, _highlighter: &SyntaxHighlighter) -> Result<Vec<DiffFile>> {
        Err(TuicrError::NoChanges)
    }
    fn fetch_context_lines(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
        _start_line: u32,
        _end_line: u32,
    ) -> Result<Vec<DiffLine>> {
        Ok(Vec::new())
    }
    fn file_line_count(
        &self,
        _file_path: &Path,
        _file_status: FileStatus,
        _ref_commit: Option<&str>,
    ) -> Result<u32> {
        Ok(0)
    }
}

fn build_app(files: Vec<DiffFile>) -> App {
    let vcs_info = VcsInfo {
        root_path: PathBuf::from("/tmp"),
        head_commit: "head".to_string(),
        branch_name: Some("main".to_string()),
        vcs_type: VcsType::Git,
    };
    let session = ReviewSession::new(
        vcs_info.root_path.clone(),
        vcs_info.head_commit.clone(),
        vcs_info.branch_name.clone(),
        SessionDiffSource::WorkingTree,
    );
    App::build(
        Box::new(DummyVcs {
            info: vcs_info.clone(),
        }),
        vcs_info,
        Theme::dark(),
        None,
        false,
        files,
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app")
}

fn make_hunk(new_start: u32, new_count: u32) -> DiffHunk {
    let mut lines = Vec::new();
    for i in 0..new_count {
        lines.push(DiffLine {
            origin: LineOrigin::Context,
            content: format!("line {}", new_start + i),
            old_lineno: Some(new_start + i),
            new_lineno: Some(new_start + i),
            highlighted_spans: None,
        });
    }
    DiffHunk {
        header: format!("@@ -{new_start},{new_count} +{new_start},{new_count} @@"),
        lines,
        old_start: new_start,
        old_count: new_count,
        new_start,
        new_count,
    }
}

fn make_file(path: &str, hunks: Vec<DiffHunk>) -> DiffFile {
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

fn thread_native_reply_positions(app: &App) -> Vec<crate::model::thread::ThreadId> {
    app.line_annotations
        .iter()
        .filter_map(|a| match a {
            AnnotatedLine::ThreadNativeReply { thread_id } => Some(thread_id.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn should_splice_a_thread_native_reply_annotation_for_a_review_level_thread() {
    // given a review-level comment whose mirrored thread gets a
    // TUI-authored reply with no legacy `Comment` counterpart
    let mut app = build_app(Vec::new());
    let comment_type = app.default_comment_type();
    let comment = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::Review,
            content: "please double-check this".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    app.session.migrate_legacy_comments_to_threads();
    let thread_id = app
        .session
        .find_thread_by_legacy_comment_id(&comment.id)
        .unwrap()
        .id()
        .clone();
    app.session
        .find_thread_mut(&thread_id)
        .unwrap()
        .thread
        .reply(crate::model::thread::ThreadComment::new(
            crate::model::thread::ThreadAuthor::human("alice"),
            "following up".to_string(),
        ));

    // when annotations are rebuilt
    app.rebuild_annotations();

    // then a `ThreadNativeReply` annotation for this thread exists,
    // immediately after the review comment's own block, with a line
    // count matching exactly what the renderer will draw.
    let expected_lines = crate::ui::comment_panel::format_thread_native_reply_lines(
        &app.theme,
        &app.session.find_thread(&thread_id).unwrap().thread,
        |id| app.session.is_legacy_comment_id(id),
        app.diff_state.viewport_width,
    )
    .len();
    assert!(expected_lines > 0);

    let positions = thread_native_reply_positions(&app);
    assert_eq!(positions.len(), expected_lines);
    assert!(positions.iter().all(|id| *id == thread_id));

    let review_comment_end = app
        .line_annotations
        .iter()
        .rposition(|a| matches!(a, AnnotatedLine::ReviewComment { comment_idx: 0 }))
        .unwrap();
    assert!(matches!(
        app.line_annotations[review_comment_end + 1],
        AnnotatedLine::ThreadNativeReply { .. }
    ));
}

#[test]
fn should_resolve_thread_id_at_cursor_directly_from_a_thread_native_reply_row() {
    // given a review comment's thread with a native-only reply, cursor
    // resting exactly on the reply's own annotation row (not the
    // original comment's row)
    let mut app = build_app(Vec::new());
    let comment_type = app.default_comment_type();
    let comment = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::Review,
            content: "root".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    app.session.migrate_legacy_comments_to_threads();
    let thread_id = app
        .session
        .find_thread_by_legacy_comment_id(&comment.id)
        .unwrap()
        .id()
        .clone();
    app.session
        .find_thread_mut(&thread_id)
        .unwrap()
        .thread
        .reply(crate::model::thread::ThreadComment::new(
            crate::model::thread::ThreadAuthor::human("alice"),
            "reply body".to_string(),
        ));
    app.rebuild_annotations();

    let reply_row = app
        .line_annotations
        .iter()
        .position(|a| matches!(a, AnnotatedLine::ThreadNativeReply { .. }))
        .expect("expected a spliced ThreadNativeReply row");
    app.diff_state.cursor_line = reply_row;

    // then `thread_id_at_cursor` resolves it directly, without any
    // `find_comment_at_cursor` / positional `CommentLocation` involved
    // (the cursor is not on the `ReviewComment` row at all).
    assert!(!matches!(
        app.line_annotations[reply_row],
        AnnotatedLine::ReviewComment { .. }
    ));
    assert_eq!(app.thread_id_at_cursor(), Some(thread_id));
}

#[test]
fn should_include_a_thread_native_navigator_item_with_status_and_author() {
    // given a review comment's thread with a native-only reply
    let mut app = build_app(Vec::new());
    let comment_type = app.default_comment_type();
    let comment = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::Review,
            content: "root".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    app.session.migrate_legacy_comments_to_threads();
    let thread_id = app
        .session
        .find_thread_by_legacy_comment_id(&comment.id)
        .unwrap()
        .id()
        .clone();
    app.session
        .find_thread_mut(&thread_id)
        .unwrap()
        .thread
        .reply(crate::model::thread::ThreadComment::new(
            crate::model::thread::ThreadAuthor::human("alice"),
            "reply body".to_string(),
        ));
    app.rebuild_annotations();

    // when building the navigator item list
    let items = app.build_comment_navigator_items();

    // then a distinct `ThreadNative` item exists (alongside the
    // `Review` item for the original comment), carrying the reply
    // author and the thread's `Open` status.
    let thread_item = items
        .iter()
        .find(|item| matches!(&item.key, CommentNavigatorKey::ThreadNative { thread_id: id } if *id == thread_id))
        .expect("expected a ThreadNative navigator item");
    assert_eq!(thread_item.author.as_deref(), Some("alice"));
    assert!(matches!(
        thread_item.kind,
        CommentNavigatorKind::Thread {
            status: crate::model::thread::ThreadStatus::Open
        }
    ));
}

#[test]
fn should_not_desync_line_annotations_after_a_native_reply_on_a_line_comment() {
    // given a file with two hunks and a line comment (+ native reply) on
    // the first hunk's line
    let hunk_a = make_hunk(10, 3);
    let hunk_b = make_hunk(50, 3);
    let file = make_file("src/lib.rs", vec![hunk_a, hunk_b]);
    let mut app = build_app(vec![file]);
    let comment_type = app.default_comment_type();

    let comment = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::Line {
                path: PathBuf::from("src/lib.rs"),
                line: 10,
                side: LineSide::New,
            },
            content: "line note".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    app.session.migrate_legacy_comments_to_threads();
    app.rebuild_annotations();
    let before_len = app.line_annotations.len();

    let thread_id = app
        .session
        .find_thread_by_legacy_comment_id(&comment.id)
        .unwrap()
        .id()
        .clone();
    app.session
        .find_thread_mut(&thread_id)
        .unwrap()
        .thread
        .reply(crate::model::thread::ThreadComment::new(
            crate::model::thread::ThreadAuthor::human("alice"),
            "native-only reply on a line comment".to_string(),
        ));
    app.rebuild_annotations();
    let after_len = app.line_annotations.len();

    let expected_extra = crate::ui::comment_panel::format_thread_native_reply_lines(
        &app.theme,
        &app.session.find_thread(&thread_id).unwrap().thread,
        |id| app.session.is_legacy_comment_id(id),
        app.diff_state.viewport_width,
    )
    .len();
    assert!(expected_extra > 0);

    // then the annotation list grew by exactly the native-reply line
    // count (previously it grew by zero — the render layer emitted the
    // extra `Line`s but `annotations.rs` never did, silently desyncing
    // every subsequent annotation's index against the rendered rows).
    assert_eq!(after_len, before_len + expected_extra);

    // and the second hunk's diff lines are still present and distinct
    // from the first hunk's — i.e. nothing after the reply got dropped
    // or merged by the splice.
    let hunk_b_diff_lines = app
        .line_annotations
        .iter()
        .filter(|a| matches!(a, AnnotatedLine::DiffLine { hunk_idx: 1, .. }))
        .count();
    assert_eq!(hunk_b_diff_lines, 3);
}

#[test]
fn should_not_splice_when_every_thread_comment_is_legacy_mirrored() {
    // given a thread whose only comment is the legacy-mirrored root (no
    // native-only reply at all) — matches the existing "avoid duplicate
    // display" contract: nothing extra should be spliced in.
    let mut app = build_app(Vec::new());
    let comment_type = app.default_comment_type();
    let comment = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::Review,
            content: "root only".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    app.session.migrate_legacy_comments_to_threads();
    assert!(
        app.session
            .find_thread_by_legacy_comment_id(&comment.id)
            .is_some()
    );

    app.rebuild_annotations();

    assert!(thread_native_reply_positions(&app).is_empty());
}

/// Finding 3 regression: a remote-imported thread has no legacy `Comment`
/// mirror at all, but its content *is* already rendered via the existing
/// `AnnotatedLine::RemoteThreadLine` path (see `push_remote_threads`).
/// `thread_id_at_cursor` must resolve the durable thread directly from
/// that row (via the provider mapping `import_remote_review_threads`
/// stamps), so reply/resolve/dismiss keybindings work on it — instead of
/// only ever working for threads that happen to have a legacy `Comment`.
mod remote_thread_cursor_resolution {
    use super::*;
    use crate::forge::remote_comments::{
        RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
    };
    use crate::forge::traits::{ForgeRepository, PrSessionKey};

    fn build_pr_app(threads: Vec<RemoteReviewThread>) -> App {
        let vcs_info = VcsInfo {
            root_path: PathBuf::from("/tmp/repo"),
            head_commit: "abcdef0123".to_string(),
            branch_name: Some("feat".to_string()),
            vcs_type: VcsType::File,
        };
        let session = ReviewSession::new(
            vcs_info.root_path.clone(),
            vcs_info.head_commit.clone(),
            vcs_info.branch_name.clone(),
            SessionDiffSource::PullRequest,
        );
        let hunk = make_hunk(10, 3);
        let file = make_file("src/lib.rs", vec![hunk]);
        let pr_source = PullRequestDiffSource {
            key: PrSessionKey::new(
                ForgeRepository::github("github.com", "acme", "widgets"),
                7,
                "abcdef0123".to_string(),
            ),
            base_sha: "0000".to_string(),
            title: "test pr".to_string(),
            url: "https://github.com/acme/widgets/pull/7".to_string(),
            head_ref_name: "feat".to_string(),
            base_ref_name: "main".to_string(),
            state: "OPEN".to_string(),
            closed: false,
            merged: false,
        };
        let mut app = App::build(
            Box::new(DummyVcs {
                info: vcs_info.clone(),
            }),
            vcs_info,
            Theme::dark(),
            None,
            false,
            vec![file],
            session,
            DiffSource::PullRequest(Box::new(pr_source)),
            InputMode::Normal,
            Vec::new(),
            None,
            None,
        )
        .expect("build pr app");
        app.forge_review_threads = threads;
        app.session
            .import_remote_review_threads("github", &app.forge_review_threads);
        app.rebuild_annotations();
        app
    }

    fn remote_thread(id: &str, line: u32) -> RemoteReviewThread {
        RemoteReviewThread {
            id: id.to_string(),
            path: "src/lib.rs".to_string(),
            line: Some(line),
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            comments: vec![RemoteReviewComment {
                id: "comment-1".to_string(),
                author: Some("octocat".to_string()),
                body: "please fix this".to_string(),
                created_at: Some(chrono::DateTime::UNIX_EPOCH),
                in_reply_to: None,
                url: String::new(),
            }],
        }
    }

    #[test]
    fn should_resolve_thread_id_at_cursor_from_a_remote_thread_line() {
        // given a remote-imported thread (no legacy Comment mirror) whose
        // content is rendered via `RemoteThreadLine`
        let mut app = build_pr_app(vec![remote_thread("gh-thread-1", 10)]);
        let expected_thread_id = app
            .session
            .find_thread_by_provider("github", "gh-thread-1")
            .expect("thread should have been imported")
            .id()
            .clone();

        let cursor_row = app
            .line_annotations
            .iter()
            .position(|a| matches!(a, AnnotatedLine::RemoteThreadLine { .. }))
            .expect("expected a RemoteThreadLine annotation");
        app.diff_state.cursor_line = cursor_row;

        // then the cursor resolves straight to the imported durable
        // thread, with no legacy comment involved at all.
        assert_eq!(app.thread_id_at_cursor(), Some(expected_thread_id));
    }

    #[test]
    fn should_dismiss_a_remote_imported_thread_from_its_remote_thread_line_row() {
        // given the cursor resting on a remote-imported thread's row
        let mut app = build_pr_app(vec![remote_thread("gh-thread-2", 10)]);
        let cursor_row = app
            .line_annotations
            .iter()
            .position(|a| matches!(a, AnnotatedLine::RemoteThreadLine { .. }))
            .expect("expected a RemoteThreadLine annotation");
        app.diff_state.cursor_line = cursor_row;

        // when dismissing via the same keybinding used for legacy threads
        let dismissed = app.dismiss_thread_at_cursor();

        // then the underlying durable thread is dismissed locally, even
        // though it was never mirrored by a legacy `Comment`.
        assert!(dismissed);
        let persisted = app
            .session
            .find_thread_by_provider("github", "gh-thread-2")
            .unwrap();
        assert_eq!(
            persisted.thread.status(),
            crate::model::thread::ThreadStatus::Dismissed
        );
    }

    #[test]
    fn should_return_none_outside_pr_mode_for_a_remote_thread_line_lookup() {
        // given a non-PR app whose cursor happens to sit at an index that
        // would be a RemoteThreadLine in PR mode — there is no provider
        // to resolve against, so this must stay a safe `None`, not panic.
        let app = build_app(Vec::new());
        assert_eq!(app.thread_id_at_cursor(), None);
    }
}
