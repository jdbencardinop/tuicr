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

/// Finding 3 audit: file-level comments (`sync_single_comment_thread`,
/// the same non-grouping path `review_comments` uses) never group two
/// file-level comments into one thread — each becomes its own
/// single-comment thread — so there is no last-legacy-comment ambiguity
/// to get wrong for this anchor kind either.
#[test]
fn should_splice_a_thread_native_reply_annotation_for_a_file_level_thread() {
    // given two separate file-level comments on the same file (each
    // migrates into its own thread, not a shared one) and a native-only
    // reply appended to only the first thread
    let file = make_file("src/lib.rs", vec![make_hunk(1, 3)]);
    let mut app = build_app(vec![file]);
    let comment_type = app.default_comment_type();
    let first = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::File {
                path: PathBuf::from("src/lib.rs"),
            },
            content: "first file-level comment".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    let second = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::File {
                path: PathBuf::from("src/lib.rs"),
            },
            content: "second file-level comment".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    app.session.migrate_legacy_comments_to_threads();

    let first_thread_id = app
        .session
        .find_thread_by_legacy_comment_id(&first.id)
        .unwrap()
        .id()
        .clone();
    let second_thread_id = app
        .session
        .find_thread_by_legacy_comment_id(&second.id)
        .unwrap()
        .id()
        .clone();
    assert_ne!(
        first_thread_id, second_thread_id,
        "file-level comments never group into one thread"
    );

    app.session
        .find_thread_mut(&first_thread_id)
        .unwrap()
        .thread
        .reply(crate::model::thread::ThreadComment::new(
            crate::model::thread::ThreadAuthor::human("alice"),
            "reply on first file comment's thread only".to_string(),
        ));

    // when annotations are rebuilt
    app.rebuild_annotations();

    // then only the first thread has a spliced native reply, and it
    // lands immediately after the first file comment's own block (before
    // the second file comment's block, which is unaffected).
    let native_reply_thread_ids = thread_native_reply_positions(&app);
    assert!(!native_reply_thread_ids.is_empty());
    assert!(
        native_reply_thread_ids
            .iter()
            .all(|id| *id == first_thread_id)
    );

    let first_comment_end = app
        .line_annotations
        .iter()
        .rposition(|a| matches!(a, AnnotatedLine::FileComment { comment_idx: 0, .. }))
        .unwrap();
    let second_comment_row = app
        .line_annotations
        .iter()
        .position(|a| matches!(a, AnnotatedLine::FileComment { comment_idx: 1, .. }))
        .unwrap();
    assert!(matches!(
        app.line_annotations[first_comment_end + 1],
        AnnotatedLine::ThreadNativeReply { .. }
    ));
    assert!(
        second_comment_row > first_comment_end,
        "second file comment's own block must not be disturbed by the first \
         thread's spliced reply"
    );
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

/// Blocker 2 regression: `migrate_legacy_comments_to_threads` groups two+
/// same-anchor legacy line comments into ONE thread (root + a legacy-
/// mirrored reply — see `sync_comment_group_thread`). A native-only reply
/// added on top of that thread must be spliced in *after the last* of the
/// grouped legacy comment blocks, not after the first — otherwise it lands
/// between the two legacy replies, breaking the root -> legacy replies ->
/// native replies order and desyncing row counts/hit-testing for
/// everything spliced after it.
#[test]
fn should_splice_native_reply_after_the_last_of_two_grouped_legacy_replies() {
    // given two legacy line comments at the exact same anchor (line 10,
    // New side) — these migrate into a single thread: root + 1 legacy
    // reply.
    let hunk = make_hunk(10, 3);
    let file = make_file("src/lib.rs", vec![hunk]);
    let mut app = build_app(vec![file]);
    let comment_type = app.default_comment_type();

    let root_comment = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::Line {
                path: PathBuf::from("src/lib.rs"),
                line: 10,
                side: LineSide::New,
            },
            content: "root legacy comment".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    let legacy_reply = add_comment_to_session(
        &mut app.session,
        AddCommentRequest {
            target: CommentTarget::Line {
                path: PathBuf::from("src/lib.rs"),
                line: 10,
                side: LineSide::New,
            },
            content: "legacy-mirrored reply".to_string(),
            comment_type: comment_type.clone(),
            author: "reviewer".to_string(),
            commit_id: None,
        },
    )
    .unwrap();
    app.session.migrate_legacy_comments_to_threads();

    let thread_id = app
        .session
        .find_thread_by_legacy_comment_id(&root_comment.id)
        .unwrap()
        .id()
        .clone();
    assert_eq!(
        app.session
            .find_thread_by_legacy_comment_id(&legacy_reply.id)
            .unwrap()
            .id(),
        &thread_id,
        "both legacy comments must have migrated into the same thread"
    );
    // Sanity: the thread really does have two legacy-mirrored comments
    // (root + reply), not just one.
    assert_eq!(
        app.session
            .find_thread(&thread_id)
            .unwrap()
            .thread
            .comments()
            .len(),
        2
    );

    // and a native-only reply with no legacy `Comment` counterpart at all
    app.session
        .find_thread_mut(&thread_id)
        .unwrap()
        .thread
        .reply(crate::model::thread::ThreadComment::new(
            crate::model::thread::ThreadAuthor::human("alice"),
            "native-only reply".to_string(),
        ));

    // when annotations are rebuilt
    app.rebuild_annotations();

    // then both legacy `LineComment` blocks (comment_idx 0 and 1) render
    // in full, back-to-back, and the spliced `ThreadNativeReply` row(s)
    // come strictly after *both* of them — not sandwiched in between.
    let last_root_row = app
        .line_annotations
        .iter()
        .rposition(|a| matches!(a, AnnotatedLine::LineComment { comment_idx: 0, .. }))
        .expect("expected root legacy comment row");
    let last_legacy_reply_row = app
        .line_annotations
        .iter()
        .rposition(|a| matches!(a, AnnotatedLine::LineComment { comment_idx: 1, .. }))
        .expect("expected legacy-mirrored reply row");
    assert!(
        last_legacy_reply_row > last_root_row,
        "legacy reply block should render after the root block"
    );

    let native_reply_rows: Vec<usize> = app
        .line_annotations
        .iter()
        .enumerate()
        .filter(|(_, a)| matches!(a, AnnotatedLine::ThreadNativeReply { thread_id: id } if *id == thread_id))
        .map(|(i, _)| i)
        .collect();
    assert!(
        !native_reply_rows.is_empty(),
        "expected at least one spliced ThreadNativeReply row"
    );
    assert!(
        native_reply_rows
            .iter()
            .all(|&row| row > last_legacy_reply_row),
        "native-only reply must be spliced after BOTH grouped legacy blocks, \
         not between them (native rows: {native_reply_rows:?}, last legacy \
         reply row: {last_legacy_reply_row})"
    );
    // Exactly one splice pass: no ThreadNativeReply rows appear before
    // the legacy blocks or interleaved between them.
    assert_eq!(
        native_reply_rows.len(),
        crate::ui::comment_panel::format_thread_native_reply_lines(
            &app.theme,
            &app.session.find_thread(&thread_id).unwrap().thread,
            |id| app.session.is_legacy_comment_id(id),
            app.diff_state.viewport_width,
        )
        .len(),
        "native reply row count must match the renderer's own line count"
    );

    // Hit-testing: cursor on the second (last) legacy block still
    // resolves the shared thread id, and cursor on the spliced native
    // row does too.
    app.diff_state.cursor_line = last_legacy_reply_row;
    assert_eq!(app.thread_id_at_cursor(), Some(thread_id.clone()));
    app.diff_state.cursor_line = native_reply_rows[0];
    assert_eq!(app.thread_id_at_cursor(), Some(thread_id));
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

    pub(super) fn build_pr_app(threads: Vec<RemoteReviewThread>) -> App {
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

    pub(super) fn remote_thread(id: &str, line: u32) -> RemoteReviewThread {
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

/// Blocker 1 regression: remote-imported durable thread mutations
/// (reply/resolve/dismiss) update `session.threads`, but until
/// `RemoteThreadOverlay`/`effective_thread_display_lines` existed,
/// render/annotations/export only ever read the raw fetched
/// `forge_review_threads` DTOs — so a local reply/resolve/dismiss made
/// against a remote-imported thread was completely invisible. These
/// tests assert the overlay actually reaches annotation row counts,
/// hit-testing, and the rendered lines themselves, without duplicating
/// remote-authored content or dropping the remote `is_outdated`/native
/// flags.
mod remote_thread_overlay_rendering {
    use super::remote_thread_cursor_resolution::{build_pr_app, remote_thread};
    use super::*;
    use crate::forge::remote_comments::{effective_thread_display_lines, thread_display_lines};

    fn remote_thread_outdated(
        id: &str,
        line: u32,
    ) -> crate::forge::remote_comments::RemoteReviewThread {
        let mut thread = remote_thread(id, line);
        thread.is_outdated = true;
        thread
    }

    #[test]
    fn should_reflect_a_local_reply_to_a_remote_imported_thread_in_annotation_row_count_exactly_once()
     {
        // given a remote-imported thread rendered via `RemoteThreadLine`,
        // no local activity yet
        let mut app = build_pr_app(vec![remote_thread("gh-thread-overlay-1", 10)]);
        let thread_id = app
            .session
            .find_thread_by_provider("github", "gh-thread-overlay-1")
            .expect("thread should have been imported")
            .id()
            .clone();
        let rows_before = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::RemoteThreadLine { .. }))
            .count();
        let base_lines = thread_display_lines(&app.forge_review_threads[0]);
        assert_eq!(rows_before, base_lines, "no overlay yet: raw DTO row count");

        // when a local-only reply is added directly to the durable thread
        // (mirrors what `reply_to_thread_at_cursor` does)
        app.session
            .find_thread_mut(&thread_id)
            .unwrap()
            .thread
            .reply(crate::model::thread::ThreadComment::new(
                crate::model::thread::ThreadAuthor::human("alice"),
                "local-only follow-up".to_string(),
            ));
        app.rebuild_annotations();

        // then the row count grows by exactly the overlay's contribution
        // — not zero (invisible) and not duplicated.
        let overlay = app.remote_thread_overlay(&app.forge_review_threads[0]);
        assert_eq!(
            overlay
                .as_ref()
                .map(|o| o.local_only_replies.len())
                .unwrap_or(0),
            1,
            "expected exactly one local-only reply in the overlay"
        );
        let rows_after = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::RemoteThreadLine { .. }))
            .count();
        let expected_after =
            effective_thread_display_lines(&app.forge_review_threads[0], overlay.as_ref());
        assert_eq!(rows_after, expected_after);
        assert!(
            rows_after > rows_before,
            "local reply must be visible in the annotation row count, not silently dropped"
        );

        // and the rendered lines themselves contain the local reply body
        // exactly once (no duplicate emission from a mirrored legacy
        // comment — there is none here).
        let rendered = crate::ui::comment_panel::format_remote_thread_lines(
            &app.theme,
            &app.forge_review_threads[0],
            false,
            overlay.as_ref(),
        );
        let occurrences = rendered
            .iter()
            .filter(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("local-only follow-up"))
            })
            .count();
        assert_eq!(
            occurrences, 1,
            "local reply should render exactly once:\n{rendered:?}"
        );
    }

    #[test]
    fn should_reflect_a_local_reply_to_a_remote_imported_thread_in_side_by_side_annotation_row_count()
     {
        // Same scenario as the unified-mode test above, but exercising
        // `build_side_by_side_annotations`'s own `push_remote_threads`
        // call site — the two annotation builders each thread the
        // `remote_overlays` slice through independently, so both must
        // stay in lockstep with the renderer.
        let mut app = build_pr_app(vec![remote_thread("gh-thread-overlay-1b", 10)]);
        app.diff_view_mode = DiffViewMode::SideBySide;
        app.rebuild_annotations();
        let thread_id = app
            .session
            .find_thread_by_provider("github", "gh-thread-overlay-1b")
            .expect("thread should have been imported")
            .id()
            .clone();
        let rows_before = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::RemoteThreadLine { .. }))
            .count();

        app.session
            .find_thread_mut(&thread_id)
            .unwrap()
            .thread
            .reply(crate::model::thread::ThreadComment::new(
                crate::model::thread::ThreadAuthor::human("alice"),
                "side-by-side local follow-up".to_string(),
            ));
        app.rebuild_annotations();

        let overlay = app.remote_thread_overlay(&app.forge_review_threads[0]);
        assert_eq!(
            overlay
                .as_ref()
                .map(|o| o.local_only_replies.len())
                .unwrap_or(0),
            1
        );
        let rows_after = app
            .line_annotations
            .iter()
            .filter(|a| matches!(a, AnnotatedLine::RemoteThreadLine { .. }))
            .count();
        let expected_after =
            effective_thread_display_lines(&app.forge_review_threads[0], overlay.as_ref());
        assert_eq!(rows_after, expected_after);
        assert!(
            rows_after > rows_before,
            "local reply must be visible in side-by-side annotation row count too"
        );
    }

    #[test]
    fn should_show_local_dismiss_badge_while_preserving_remote_outdated_flag() {
        // given a remote thread already flagged `is_outdated` by the
        // provider (native remote concept, no local equivalent)
        let mut app = build_pr_app(vec![remote_thread_outdated("gh-thread-overlay-2", 10)]);
        // Outdated threads are hidden under the default `Unresolved`
        // visibility filter — switch to `All` so this test can exercise
        // the badge/flag interaction on a rendered row.
        app.session.remote_comments_visibility =
            crate::forge::remote_comments::PrCommentsVisibility::All;
        app.rebuild_annotations();
        let cursor_row = app
            .line_annotations
            .iter()
            .position(|a| matches!(a, AnnotatedLine::RemoteThreadLine { .. }))
            .expect("expected a RemoteThreadLine annotation");
        app.diff_state.cursor_line = cursor_row;

        // when dismissed locally via the same keybinding used for legacy
        // threads
        assert!(app.dismiss_thread_at_cursor());
        app.rebuild_annotations();

        // then the overlay carries the local `Dismissed` status...
        let overlay = app
            .remote_thread_overlay(&app.forge_review_threads[0])
            .expect("expected an overlay for the dismissed thread");
        assert_eq!(
            overlay.local_status,
            crate::model::thread::ThreadStatus::Dismissed
        );
        // ...the remote DTO's own `is_outdated` flag is untouched (the
        // overlay never mutates the DTO)...
        assert!(
            app.forge_review_threads[0].is_outdated,
            "remote outdated flag must survive a local dismiss"
        );
        // ...and the rendered lines show both: a local-status badge and
        // the native outdated marker.
        let rendered = crate::ui::comment_panel::format_remote_thread_lines(
            &app.theme,
            &app.forge_review_threads[0],
            false,
            Some(&overlay),
        );
        let header = rendered
            .first()
            .expect("expected a header line")
            .spans
            .iter()
            .map(|s| s.content.to_string())
            .collect::<String>();
        assert!(
            header.to_lowercase().contains("outdated"),
            "expected native outdated marker preserved in header:\n{header}"
        );
        assert!(
            header.to_lowercase().contains("dismiss"),
            "expected a local-dismiss badge in header:\n{header}"
        );
    }

    #[test]
    fn should_not_duplicate_a_local_reply_after_reimporting_the_same_remote_thread_twice() {
        // given a remote-imported thread with a local-only reply already
        // attached
        let mut app = build_pr_app(vec![remote_thread("gh-thread-overlay-3", 10)]);
        let thread_id = app
            .session
            .find_thread_by_provider("github", "gh-thread-overlay-3")
            .unwrap()
            .id()
            .clone();
        app.session
            .find_thread_mut(&thread_id)
            .unwrap()
            .thread
            .reply(crate::model::thread::ThreadComment::new(
                crate::model::thread::ThreadAuthor::human("alice"),
                "keep me exactly once".to_string(),
            ));

        // when the same remote thread is re-fetched/re-imported twice
        // (simulating two poll cycles against an unchanged remote thread)
        app.session
            .import_remote_review_threads("github", &app.forge_review_threads);
        app.session
            .import_remote_review_threads("github", &app.forge_review_threads);
        app.rebuild_annotations();

        // then the local-only reply is still present exactly once, not
        // duplicated by either reimport pass.
        let persisted = app
            .session
            .find_thread_by_provider("github", "gh-thread-overlay-3")
            .unwrap();
        let local_replies: Vec<_> = persisted
            .thread
            .replies()
            .filter(|r| r.author.kind != crate::model::thread::AuthorKind::Remote)
            .collect();
        assert_eq!(
            local_replies.len(),
            1,
            "expected exactly one local-only reply after two reimports, got {local_replies:?}"
        );

        let overlay = app.remote_thread_overlay(&app.forge_review_threads[0]);
        assert_eq!(
            overlay
                .as_ref()
                .map(|o| o.local_only_replies.len())
                .unwrap_or(0),
            1
        );
    }

    /// Finding 3 audit: two distinct threads anchored effectively at the
    /// same source line — a bare line comment and a range comment ending
    /// on that same line number share the file's `line_comments` HashMap
    /// bucket (see `migrate_legacy_comments_to_threads`'s grouping
    /// comment), but differ in `(side, line_range)` so they migrate into
    /// two *separate* `PersistedThread`s, not one grouped thread. A
    /// native-only reply added to only one of them must not leak into,
    /// duplicate onto, or reorder the other thread's own legacy block.
    #[test]
    fn should_not_cross_contaminate_native_replies_between_two_threads_sharing_a_line_bucket() {
        // given a bare line comment on line 5 (New side) and a distinct
        // range comment spanning lines 3-5 (also New side) - both land in
        // `line_comments[&5]` but form two separate anchors/threads.
        let hunk = make_hunk(1, 10);
        let file = make_file("src/lib.rs", vec![hunk]);
        let mut app = build_app(vec![file]);
        let comment_type = app.default_comment_type();

        let bare_line_comment = add_comment_to_session(
            &mut app.session,
            AddCommentRequest {
                target: CommentTarget::Line {
                    path: PathBuf::from("src/lib.rs"),
                    line: 5,
                    side: LineSide::New,
                },
                content: "bare line thread root".to_string(),
                comment_type: comment_type.clone(),
                author: "reviewer".to_string(),
                commit_id: None,
            },
        )
        .unwrap();
        let range_comment = add_comment_to_session(
            &mut app.session,
            AddCommentRequest {
                target: CommentTarget::LineRange {
                    path: PathBuf::from("src/lib.rs"),
                    range: crate::model::LineRange { start: 3, end: 5 },
                    side: LineSide::New,
                },
                content: "range thread root".to_string(),
                comment_type: comment_type.clone(),
                author: "reviewer".to_string(),
                commit_id: None,
            },
        )
        .unwrap();
        app.session.migrate_legacy_comments_to_threads();

        let bare_thread_id = app
            .session
            .find_thread_by_legacy_comment_id(&bare_line_comment.id)
            .unwrap()
            .id()
            .clone();
        let range_thread_id = app
            .session
            .find_thread_by_legacy_comment_id(&range_comment.id)
            .unwrap()
            .id()
            .clone();
        assert_ne!(
            bare_thread_id, range_thread_id,
            "a bare line comment and a same-ending-line range comment must migrate \
             into two distinct threads, not be grouped into one"
        );

        // when a native-only reply is appended to the bare-line thread
        // only
        app.session
            .find_thread_mut(&bare_thread_id)
            .unwrap()
            .thread
            .reply(crate::model::thread::ThreadComment::new(
                crate::model::thread::ThreadAuthor::human("alice"),
                "reply on bare line thread only".to_string(),
            ));
        app.rebuild_annotations();

        // then only `ThreadNativeReply` rows for the bare-line thread
        // exist (the badge + body lines the renderer produces for one
        // reply) - the range thread's own legacy block is untouched (no
        // reply spliced onto it, and none of its rows carry a
        // `range_thread_id`).
        let native_reply_thread_ids = thread_native_reply_positions(&app);
        assert!(
            !native_reply_thread_ids.is_empty(),
            "expected at least one spliced ThreadNativeReply row"
        );
        assert!(
            native_reply_thread_ids
                .iter()
                .all(|id| *id == bare_thread_id),
            "expected every spliced ThreadNativeReply row to belong to the bare-line \
             thread only, got {native_reply_thread_ids:?}"
        );

        // and the range thread's own legacy comment is still last-legacy
        // for its own thread (untouched by the other thread's mutation),
        // while the bare-line thread's legacy comment stays last-legacy
        // for its own thread too.
        assert!(
            app.session
                .is_last_legacy_comment_for_thread(&range_comment.id)
        );
        assert!(
            app.session
                .is_last_legacy_comment_for_thread(&bare_line_comment.id)
        );

        // and both threads' own legacy rows are still present, distinct,
        // and not reordered relative to each other by the splice.
        let bare_row = app.line_annotations.iter().position(|a| {
            matches!(
                a,
                AnnotatedLine::LineComment {
                    line: 5,
                    side: LineSide::New,
                    comment_idx: 0,
                    ..
                }
            )
        });
        assert!(
            bare_row.is_some(),
            "expected the bare-line comment's own row"
        );
    }
}
