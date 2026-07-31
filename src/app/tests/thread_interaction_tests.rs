//! Requirement 3 regression tests: TUI durable-thread interactions
//! (add/reply/resolve/reopen/dismiss) routed through `ReviewSession`/
//! `Thread` invariants rather than raw field mutation, plus the
//! dual-write that mints a canonical `Thread` alongside every newly
//! created legacy `Comment` (see `App::mirror_new_comment_as_thread`).

use crate::app::*;
use crate::model::thread::{AnchorTarget, ThreadStatus};
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

fn build_app() -> App {
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
        Vec::new(),
        session,
        DiffSource::WorkingTree,
        InputMode::Normal,
        Vec::new(),
        None,
        None,
    )
    .expect("failed to build test app")
}

/// Move the cursor to wherever `rebuild_annotations` placed the review
/// comment at `idx`, mirroring how a real user would navigate there
/// before pressing `t`/`x`/`X`.
fn cursor_to_review_comment(app: &mut App, idx: usize) {
    let line = app
        .line_annotations
        .iter()
        .position(
            |a| matches!(a, AnnotatedLine::ReviewComment { comment_idx } if *comment_idx == idx),
        )
        .expect("review comment annotation not found");
    app.diff_state.cursor_line = line;
}

fn add_review_comment(app: &mut App, body: &str) {
    app.enter_review_comment_mode();
    app.comment_buffer = body.to_string();
    app.save_comment();
    app.rebuild_annotations();
}

#[test]
fn should_mint_canonical_thread_alongside_new_review_comment() {
    // given an app with no threads yet
    let mut app = build_app();
    assert!(app.session.threads.is_empty());

    // when a new review-level comment is saved through the normal
    // comment-entry flow
    add_review_comment(&mut app, "please double-check this");

    // then a legacy Comment AND a canonical Thread both exist, so
    // reply/resolve/dismiss can find it via `thread_id_at_cursor`.
    assert_eq!(app.session.review_comments.len(), 1);
    assert_eq!(app.session.threads.len(), 1);
    let persisted = &app.session.threads[0];
    assert_eq!(persisted.thread.status(), ThreadStatus::Open);
    assert_eq!(persisted.thread.anchor().target(), &AnchorTarget::Review);
    assert_eq!(
        persisted.thread.root().unwrap().body,
        "please double-check this"
    );
}

#[test]
fn should_not_duplicate_thread_when_editing_existing_review_comment() {
    // given a review comment that already minted its thread
    let mut app = build_app();
    add_review_comment(&mut app, "first draft");
    assert_eq!(app.session.threads.len(), 1);

    // when the same comment is edited in place (not a new comment)
    cursor_to_review_comment(&mut app, 0);
    app.enter_review_comment_mode();
    app.editing_comment_id = Some(app.session.review_comments[0].id.clone());
    app.comment_buffer = "edited draft".to_string();
    app.save_comment();

    // then the legacy comment content changed but no second thread was
    // minted — dual-write only applies to genuinely new comments.
    assert_eq!(app.session.review_comments[0].content, "edited draft");
    assert_eq!(app.session.threads.len(), 1);
}

#[test]
fn should_reply_to_thread_at_cursor_via_thread_native_reply() {
    // given a review comment (and its mirrored thread) under the cursor
    let mut app = build_app();
    add_review_comment(&mut app, "initial comment");
    cursor_to_review_comment(&mut app, 0);

    // when replying via the `t` action's handler
    let saved = app.reply_to_thread_at_cursor("following up".to_string());

    // then the reply lands on the thread as a genuine reply, not a
    // second legacy comment.
    assert!(saved);
    assert_eq!(app.session.review_comments.len(), 1);
    let persisted = &app.session.threads[0];
    assert_eq!(persisted.thread.comments().len(), 2);
    assert_eq!(
        persisted.thread.replies().next().unwrap().body,
        "following up"
    );
}

#[test]
fn should_toggle_thread_resolved_and_reopened_at_cursor() {
    // given a review comment's thread under the cursor, initially Open
    let mut app = build_app();
    add_review_comment(&mut app, "needs a look");
    cursor_to_review_comment(&mut app, 0);
    assert_eq!(app.session.threads[0].thread.status(), ThreadStatus::Open);

    // when toggling resolved
    assert!(app.toggle_thread_resolved_at_cursor());
    assert_eq!(
        app.session.threads[0].thread.status(),
        ThreadStatus::Resolved
    );

    // and toggling again reopens it
    assert!(app.toggle_thread_resolved_at_cursor());
    assert_eq!(app.session.threads[0].thread.status(), ThreadStatus::Open);
}

#[test]
fn should_dismiss_thread_at_cursor_and_freeze_it_terminally() {
    // given a review comment's thread under the cursor
    let mut app = build_app();
    add_review_comment(&mut app, "won't fix candidate");
    cursor_to_review_comment(&mut app, 0);

    // when dismissing it
    assert!(app.dismiss_thread_at_cursor());
    assert_eq!(
        app.session.threads[0].thread.status(),
        ThreadStatus::Dismissed
    );

    // then it can never be resolved or reopened again (terminal, per
    // `Thread::resolve`/`Thread::reopen`'s documented contract) — the
    // toggle handler still finds the thread (returns `true`) but its
    // status stays frozen at `Dismissed`.
    assert!(app.toggle_thread_resolved_at_cursor());
    assert_eq!(
        app.session.threads[0].thread.status(),
        ThreadStatus::Dismissed
    );
    assert_eq!(
        app.message.as_ref().map(|m| m.content.as_str()),
        Some("Dismissed threads cannot be resolved")
    );
}

#[test]
fn should_show_honest_message_when_no_thread_is_at_cursor() {
    // given an app with nothing under the cursor
    let mut app = build_app();

    // when attempting thread actions with no comment/thread at cursor
    assert!(!app.reply_to_thread_at_cursor("x".to_string()));
    assert!(!app.toggle_thread_resolved_at_cursor());
    assert!(!app.dismiss_thread_at_cursor());

    // then each surfaces an honest "nothing to act on" message rather
    // than silently no-op-ing.
    assert!(
        app.message
            .as_ref()
            .map(|m| m.content.contains("Move cursor"))
            .unwrap_or(false)
    );
}
