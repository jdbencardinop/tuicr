//! TUI-side wiring for durable [`crate::model::thread::Thread`]s: real
//! anchor refresh after a PR head advance, and cursor-driven
//! reply/resolve/reopen/dismiss actions that route through `Thread`'s own
//! invariants (never a raw field mutation).

use super::*;
use crate::forge::context::ContextProvider;
use crate::model::thread::{AuthorKind, ThreadAuthor, ThreadComment, ThreadId, ThreadStatus};

impl App {
    /// Mint a canonical [`crate::model::thread::Thread`] mirroring every
    /// legacy `Comment` (review/file/line/range) that doesn't have one yet,
    /// so every new comment created through the existing comment-entry UI
    /// also becomes durable-thread canonical state (see module docs and
    /// requirement: "TUI canonical state is threads; legacy APIs remain
    /// compatible").
    ///
    /// Delegates to [`crate::model::review::ReviewSession::migrate_legacy_comments_to_threads`]
    /// rather than minting an unrelated thread via `add_thread_to_session`:
    /// that migration path derives each thread's (and its root comment's)
    /// ID deterministically *from the legacy `Comment.id` itself*, which is
    /// exactly what lets `find_thread_by_legacy_comment_id`/
    /// `thread_id_at_cursor` resolve a thread for a comment created this
    /// way. It is also already idempotent (a repeat call for an
    /// already-migrated comment is a no-op) and already groups multiple
    /// legacy comments at the same line/range anchor into one thread
    /// (root + replies) instead of one-thread-per-comment, so calling it
    /// after every save is safe and cheap for interactive TUI use.
    pub(in crate::app) fn mirror_new_comments_as_threads(&mut self) {
        self.session.migrate_legacy_comments_to_threads();
    }

    /// Re-run anchor relocation for every carried-forward thread whose
    /// anchor targets a file present in the currently-open PR's diff,
    /// using the real new-side file content (via [`Self::context_provider`]).
    ///
    /// Threads anchored at [`crate::model::thread::AnchorTarget::Review`]
    /// (no file) or at a file no longer part of the diff are left
    /// untouched: there is no new content to relocate against, and leaving
    /// a thread's prior anchor state as-is is honest (never a false
    /// "still current"). Closed threads (`Resolved`/`Dismissed`) are
    /// already no-ops inside `refresh_anchor_with_remap` (see
    /// `ThreadAnchorRefresh::Frozen`), so this walks every thread rather
    /// than pre-filtering by status.
    ///
    /// Called only where an `App` instance (and therefore a
    /// `context_provider()`) already exists — i.e. the already-running PR
    /// reload paths (`finish_pr_reload`, `reload_pull_request_with_backend`).
    /// The cold-start path (`opened_pr_with_persisted_session`, run before
    /// an `App` exists) still carries every thread forward via
    /// `reviewed_state_carried_forward` but cannot refresh anchors yet —
    /// an acknowledged, honest limitation: no thread is ever lost, but its
    /// anchor state may be momentarily stale until the next call here.
    pub(in crate::app) fn refresh_thread_anchors_after_head_advance(&mut self) {
        if self.session.threads.is_empty() {
            return;
        }

        let mut content_by_path: HashMap<PathBuf, Vec<String>> = HashMap::new();
        {
            let file_by_path: HashMap<PathBuf, &DiffFile> = self
                .diff_files
                .iter()
                .map(|file| (file.display_path().clone(), file))
                .collect();
            let referenced_paths: std::collections::BTreeSet<PathBuf> = self
                .session
                .threads
                .iter()
                .filter_map(|persisted| {
                    persisted.thread.anchor().target().path().map(PathBuf::from)
                })
                .filter(|path| file_by_path.contains_key(path))
                .collect();

            let provider = self.context_provider();
            for path in &referenced_paths {
                let diff_file = file_by_path[path];
                if let Some(lines) = Self::fetch_full_file_lines(provider.as_ref(), diff_file) {
                    content_by_path.insert(path.clone(), lines);
                }
            }
        }

        for persisted in self.session.threads.iter_mut() {
            let Some(path_str) = persisted.thread.anchor().target().path() else {
                continue;
            };
            let path = PathBuf::from(path_str);
            let Some(lines) = content_by_path.get(&path) else {
                continue;
            };
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            let _ = persisted.refresh_anchor_with_remap(&refs, None);
        }
    }

    /// Fetch every line of `diff_file` at the revision `provider` resolves
    /// to (the new/head side for additions/modifications, the old/base
    /// side for pure deletions — see `ContextProvider` implementations),
    /// as plain strings suitable for `Anchor::relocate`/
    /// `refresh_anchor_with_remap`. Returns `None` if the fetch fails
    /// (e.g. transient network error against a forge); callers leave the
    /// affected threads' anchors untouched rather than treating a fetch
    /// failure as "file is empty" (which would falsely mark every anchor
    /// in it `Stale`).
    fn fetch_full_file_lines(
        provider: &dyn ContextProvider,
        diff_file: &DiffFile,
    ) -> Option<Vec<String>> {
        let old_path = diff_file.old_path.as_ref();
        let new_path = diff_file.new_path.as_ref();
        let total = provider
            .file_line_count(old_path, new_path, diff_file.status)
            .ok()?;
        if total == 0 {
            return Some(Vec::new());
        }
        let lines = provider
            .fetch_context_lines(old_path, new_path, diff_file.status, 1, total)
            .ok()?;
        Some(lines.into_iter().map(|line| line.content).collect())
    }

    /// Resolve the durable thread under the cursor. Tries, in order:
    /// 1. A thread-native annotation row
    ///    ([`crate::app::AnnotatedLine::ThreadNativeReply`]) that carries
    ///    its `ThreadId` directly.
    /// 2. A rendered remote-thread row
    ///    ([`crate::app::AnnotatedLine::RemoteThreadLine`]) — resolved via
    ///    the imported durable thread's provider mapping, so replying to
    ///    or resolving/dismissing a remote-imported thread works even
    ///    though it has no legacy `Comment` mirror at all (see
    ///    [`Self::thread_id_for_remote_thread`]).
    /// 3. The legacy-mirrored comment/root row under the cursor, via the
    ///    legacy comment it mirrors.
    ///
    /// Checking the thread-native paths first means a cursor resting on
    /// content with no legacy `Comment` counterpart (a TUI-authored
    /// reply, or a remote-imported thread/reply) still resolves
    /// correctly, instead of only ever falling through
    /// `find_comment_at_cursor`'s positional `CommentLocation` lookup.
    pub(in crate::app) fn thread_id_at_cursor(&self) -> Option<ThreadId> {
        if let Some(AnnotatedLine::ThreadNativeReply { thread_id }) =
            self.line_annotations.get(self.diff_state.cursor_line)
        {
            return Some(thread_id.clone());
        }

        if let Some(AnnotatedLine::RemoteThreadLine { thread_idx }) =
            self.line_annotations.get(self.diff_state.cursor_line)
        {
            return self.thread_id_for_remote_thread(*thread_idx);
        }

        let location = self.find_comment_at_cursor()?;
        let comment_id = match location {
            CommentLocation::Review { index } => {
                self.session.review_comments.get(index)?.id.clone()
            }
            CommentLocation::File { path, index } => self
                .session
                .files
                .get(&path)?
                .file_comments
                .get(index)?
                .id
                .clone(),
            CommentLocation::Line {
                path, line, index, ..
            } => self
                .session
                .files
                .get(&path)?
                .line_comments
                .get(&line)?
                .get(index)?
                .id
                .clone(),
        };
        self.session
            .find_thread_by_legacy_comment_id(&comment_id)
            .map(|persisted| persisted.id().clone())
    }

    /// Resolve the durable [`ThreadId`] that
    /// `import_remote_review_threads_from_current_pr` imported
    /// `self.forge_review_threads[thread_idx]` into, by matching on the
    /// same `(provider, provider_id)` pair `import_remote_review_threads`
    /// stamps into `provider_mappings`. `None` outside PR mode (there is
    /// no provider to key the lookup on) or if the remote thread hasn't
    /// been imported yet (e.g. import failed/hasn't run since the last
    /// fetch).
    fn thread_id_for_remote_thread(&self, thread_idx: usize) -> Option<ThreadId> {
        let remote_thread = self.forge_review_threads.get(thread_idx)?;
        self.persisted_thread_for_remote(remote_thread)
            .map(|persisted| persisted.id().clone())
    }

    /// The forge provider key for the current PR session, if any — the
    /// same key `import_remote_review_threads`/`thread_from_remote` stamp
    /// into `provider_mappings`. `None` outside PR mode.
    fn remote_provider_key(&self) -> Option<&'static str> {
        let DiffSource::PullRequest(pr) = &self.diff_source else {
            return None;
        };
        Some(pr.key.repository.kind.provider_key())
    }

    /// The durable [`crate::model::thread_store::PersistedThread`] that
    /// `remote` was imported into (matched on `(provider, remote.id)`, the
    /// same pair `import_remote_review_threads`/`thread_from_remote` stamp
    /// into `provider_mappings`). `None` outside PR mode, or if the remote
    /// thread hasn't been imported yet (e.g. mid-fetch before
    /// `import_remote_review_threads_from_current_pr` has run, or in tests
    /// that populate `forge_review_threads` directly without importing) —
    /// callers fall back to the raw remote DTO unchanged in that case.
    pub fn persisted_thread_for_remote(
        &self,
        remote: &crate::forge::remote_comments::RemoteReviewThread,
    ) -> Option<&crate::model::thread_store::PersistedThread> {
        let provider = self.remote_provider_key()?;
        self.session.find_thread_by_provider(provider, &remote.id)
    }

    /// The [`crate::forge::remote_comments::RemoteThreadOverlay`] for
    /// `remote` — local-only replies plus the durable thread's own status —
    /// so render/annotation/export call sites can show a local reply/
    /// resolve/dismiss/reopen made against the thread that mirrors this
    /// remote DTO (see this struct's doc comment for why the DTO itself is
    /// never mutated). `None` when [`Self::persisted_thread_for_remote`]
    /// finds no durable thread; callers must treat that identically to "no
    /// local activity" and render the remote DTO unchanged.
    ///
    /// Delegates to [`crate::forge::remote_comments::remote_thread_overlay_for_session`]
    /// so the TUI render path and the non-`App` export/markdown path
    /// (`generate_markdown`) can never diverge on what counts as a "local
    /// overlay" for the same `(session, remote)` pair.
    pub fn remote_thread_overlay(
        &self,
        remote: &crate::forge::remote_comments::RemoteReviewThread,
    ) -> Option<crate::forge::remote_comments::RemoteThreadOverlay> {
        crate::forge::remote_comments::remote_thread_overlay_for_session(
            &self.session,
            self.remote_provider_key(),
            remote,
        )
    }

    /// Enter comment-input mode composing a reply to the thread at cursor
    /// (`t`). Falls back to a message if there is no comment/thread under
    /// the cursor. Reuses the same `InputMode::Comment` text editor as
    /// legacy comment entry; `save_comment` special-cases
    /// `thread_reply_target` to route the submitted text into
    /// `Thread::reply` instead of a new legacy `Comment`.
    pub fn enter_thread_reply_mode(&mut self) {
        let Some(thread_id) = self.thread_id_at_cursor() else {
            self.set_message("Move cursor to a comment/thread to reply");
            return;
        };
        self.input_mode = InputMode::Comment;
        self.diff_state.scroll_x = 0;
        self.comment_buffer.clear();
        self.comment_cursor = 0;
        self.comment_type = self.default_comment_type();
        self.comment_is_review_level = false;
        self.comment_is_file_level = false;
        self.comment_line = None;
        self.comment_line_range = None;
        self.editing_comment_id = None;
        self.thread_reply_target = Some(thread_id);
    }

    /// Reply to the thread at the cursor with `body`, appended as a
    /// genuinely thread-native reply (via `Thread::reply`, not a mirrored
    /// legacy `Comment`) so it applies uniformly to review/file/line/range
    /// anchored threads alike, including the review/file-level anchors
    /// whose legacy grouping is always "1 comment = 1 thread" (see
    /// `ReviewSession::migrate_legacy_comments_to_threads`) and therefore
    /// has no legacy-comment reply concept of its own.
    ///
    /// Resolves the target from the *current* cursor position — safe only
    /// for callers that resolve and commit in the same call with no
    /// intervening state (e.g. a direct test helper or a future
    /// immediate, non-text-composing keybinding). `App::save_comment`'s
    /// `t`-composed reply flow does **not** use this: it captures the
    /// `ThreadId` up front in `enter_thread_reply_mode` and must commit
    /// against that captured id via [`Self::reply_to_thread`] instead,
    /// since the cursor can move (scrolling, an autosave-triggered
    /// `rebuild_annotations`, an external merge reordering threads, etc.)
    /// during the arbitrarily-long text-composition gap between entering
    /// reply mode and pressing save.
    pub fn reply_to_thread_at_cursor(&mut self, body: String) -> bool {
        let Some(thread_id) = self.thread_id_at_cursor() else {
            self.set_message("Move cursor to a comment/thread to reply");
            return false;
        };
        self.reply_to_thread(&thread_id, body)
    }

    /// Reply to the thread identified by the explicit `thread_id` — never
    /// re-resolved from the cursor — with `body`, appended as a genuinely
    /// thread-native reply (via `Thread::reply`, not a mirrored legacy
    /// `Comment`). This is the target-safe primitive
    /// [`Self::reply_to_thread_at_cursor`] and `App::save_comment`'s
    /// `thread_reply_target`-driven reply both delegate to; the cursor is
    /// never consulted here. If `thread_id` no longer resolves to a thread
    /// (e.g. it was somehow removed between capture and save — not
    /// currently possible via any TUI action, but never assumed), this
    /// reports an explicit "Thread no longer exists" error and returns
    /// `false` rather than silently falling back to whatever thread is
    /// now under the cursor.
    pub fn reply_to_thread(&mut self, thread_id: &ThreadId, body: String) -> bool {
        let Some(persisted) = self.session.find_thread_mut(thread_id) else {
            self.set_message("Thread no longer exists");
            return false;
        };
        persisted.thread.reply(ThreadComment::new(
            ThreadAuthor::human(self.username.clone()),
            body,
        ));
        self.dirty = true;
        if let Err(e) = self.save_current_session_merging_external() {
            self.set_error(format!("Reply saved locally but autosave failed: {e}"));
        } else {
            self.set_message("Reply added to thread");
        }
        self.rebuild_annotations();
        true
    }

    /// Toggle the thread at the cursor between `Resolved` and reopened
    /// (re-deriving `Stale`/`Ambiguous`/`Open` from the anchor, never a
    /// forced `Open` — see `Thread::reopen`). No-op (with a message) if
    /// there is no thread at the cursor, or if it is terminally
    /// `Dismissed`.
    pub fn toggle_thread_resolved_at_cursor(&mut self) -> bool {
        let Some(thread_id) = self.thread_id_at_cursor() else {
            self.set_message("Move cursor to a comment/thread to resolve/reopen");
            return false;
        };
        let Some(persisted) = self.session.find_thread_mut(&thread_id) else {
            self.set_message("Thread no longer exists");
            return false;
        };
        let message = if persisted.thread.is_resolved() {
            if persisted.thread.reopen() {
                "Thread reopened"
            } else {
                "Thread could not be reopened"
            }
        } else if persisted.thread.resolve() {
            "Thread resolved"
        } else {
            "Dismissed threads cannot be resolved"
        };
        self.dirty = true;
        if let Err(e) = self.save_current_session_merging_external() {
            self.set_error(format!("{message}, but autosave failed: {e}"));
        } else {
            self.set_message(message);
        }
        self.rebuild_annotations();
        true
    }

    /// Dismiss ("won't fix") the thread at the cursor. Terminal: the
    /// thread can never be resolved or reopened again afterward (see
    /// `Thread::dismiss`).
    pub fn dismiss_thread_at_cursor(&mut self) -> bool {
        let Some(thread_id) = self.thread_id_at_cursor() else {
            self.set_message("Move cursor to a comment/thread to dismiss");
            return false;
        };
        let Some(persisted) = self.session.find_thread_mut(&thread_id) else {
            self.set_message("Thread no longer exists");
            return false;
        };
        persisted.thread.dismiss();
        self.dirty = true;
        if let Err(e) = self.save_current_session_merging_external() {
            self.set_error(format!("Thread dismissed but autosave failed: {e}"));
        } else {
            self.set_message("Thread dismissed");
        }
        self.rebuild_annotations();
        true
    }

    /// Whether the session holds any durable-thread activity that
    /// `:submit`'s network call cannot see — since that path (see
    /// `App::spawn_pr_submit`) walks legacy `Comment`s only, req. 6's
    /// "local thread publication isn't yet wired" applies to two kinds of
    /// content: a comment/reply with no legacy `Comment` counterpart
    /// *and* not already authored by the provider itself (see
    /// [`Self::thread_has_unpublished_comment`]), and a locally-driven
    /// `Resolved`/`Dismissed` status that the provider does not already
    /// independently reflect (see [`Self::thread_status_is_unpublished`]).
    ///
    /// Deliberately does **not** flag a remote-imported thread's own
    /// root/replies (`AuthorKind::Remote`) or a `Resolved` status that
    /// merely mirrors the provider's own `is_resolved` flag captured at
    /// import time (`PersistedThread::provider_mappings`) — that content
    /// already exists upstream verbatim; nothing about it is "unpublished
    /// local activity" the user did in this TUI session. Only a genuine
    /// local reply/resolve/dismiss (or a purely local, never-imported
    /// thread) should surface the submit-modal warning.
    pub fn has_unpublished_thread_activity(&self) -> bool {
        self.session.threads.iter().any(|persisted| {
            Self::thread_status_is_unpublished(persisted)
                || self.thread_has_unpublished_comment(persisted)
        })
    }

    /// Whether `persisted`'s current [`ThreadStatus`] represents a local
    /// resolve/dismiss action not already reflected by any provider this
    /// thread has been imported from/mapped to.
    ///
    /// - `Dismissed` has no provider-native equivalent at all (see
    ///   `forge::dryrun::plan_thread`'s identical reasoning), so it is
    ///   always "unpublished" while the thread carries that status.
    /// - `Resolved` is compared against the baseline captured in
    ///   `provider_mappings` (`thread_from_remote` stamps `"is_resolved"`
    ///   there at import time, and re-imports keep it current — see
    ///   `merge_remote_thread_into_existing`): if *any* mapped provider
    ///   already reports `is_resolved: true`, this thread's `Resolved`
    ///   status merely mirrors that remote truth (e.g. an untouched
    ///   already-resolved imported thread) and is not local-only activity.
    ///   A thread with no provider mapping at all (never imported/
    ///   published) always counts as unpublished once resolved.
    /// - `Open`/`Stale`/`Ambiguous` are never "unpublished thread
    ///   activity": `Open` is the default, no-action state, and
    ///   `Stale`/`Ambiguous` are anchor-relocation outcomes the user did
    ///   not choose, not a resolve/dismiss action to publish.
    fn thread_status_is_unpublished(
        persisted: &crate::model::thread_store::PersistedThread,
    ) -> bool {
        match persisted.thread.status() {
            ThreadStatus::Dismissed => true,
            ThreadStatus::Resolved => !persisted
                .provider_mappings
                .values()
                .any(|mapping| mapping.get("is_resolved").and_then(|v| v.as_bool()) == Some(true)),
            ThreadStatus::Open | ThreadStatus::Stale | ThreadStatus::Ambiguous => false,
        }
    }

    /// Whether `persisted` carries a comment/reply with no legacy
    /// `Comment` mirror (see `ReviewSession::is_legacy_comment_id`) that
    /// was also authored locally rather than fetched from a provider
    /// (`author.kind != AuthorKind::Remote`, the same convention
    /// `forge::dryrun::plan_thread`'s reply-planning loop and
    /// `RemoteThreadOverlay::local_only_replies` both use to identify
    /// "not already represented on the provider" content).
    ///
    /// Both conditions are required: a legacy-mirrored comment (added via
    /// the ordinary comment-entry UI, human/agent-authored) already gets
    /// submitted through the existing legacy `Comment` walk, so it is not
    /// "thread-only" unpublished content; a remote-imported root/reply
    /// (`AuthorKind::Remote`) has no legacy mirror either, but it already
    /// exists on the provider verbatim, so it is not local activity to
    /// warn about at all. Only a reply added natively via the TUI's
    /// thread-reply keybinding — whether on a purely local thread or on
    /// top of an already-imported one — satisfies both.
    fn thread_has_unpublished_comment(
        &self,
        persisted: &crate::model::thread_store::PersistedThread,
    ) -> bool {
        persisted.thread.comments().iter().any(|comment| {
            comment.author.kind != AuthorKind::Remote
                && !self.session.is_legacy_comment_id(comment.id().as_str())
        })
    }
}
