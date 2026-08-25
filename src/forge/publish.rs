//! Durable-thread mutation executor.
//!
//! This is the write side of [`crate::forge::dryrun::plan_publication`]:
//! [`execute_plan`] walks the exact same [`DryRunPlan`] a caller would have
//! shown the user in a `--dry-run` preview, and — for every operation whose
//! outcome is [`OperationOutcome::Planned`] or [`OperationOutcome::Emulated`]
//! — issues the corresponding [`ForgeBackend`] call, persisting each
//! success into `session`'s [`PersistedThread::provider_mappings`] before
//! moving on to the next operation.
//!
//! ## Idempotency and safe resume
//!
//! `execute_plan` itself does not need its own separate "resume" state
//! machine: every successful operation immediately updates the exact same
//! `provider_mappings`/reply-ledger/root-comment-ledger state that
//! `plan_publication`'s own dedup logic reads (see `dryrun.rs`'s module
//! doc comment). So the safe-resume story is simply: on a hard failure,
//! stop (leaving the completed operations' state durably recorded and the
//! remaining operations unexecuted), then have the caller re-run
//! `plan_publication` against the now-partially-updated `session` and
//! `execute_plan` the smaller resulting plan — every already-completed
//! operation is naturally absent from that new plan, and nothing already
//! published is ever resent.
//!
//! The TUI uses checkpointed execution for non-draft GitLab publication.

use crate::error::{Result, TuicrError};
use crate::forge::dryrun::{OperationKind, OperationOutcome, PlannedOperation};
use crate::forge::submit::SubmitEvent;
use crate::forge::traits::{
    CreateReviewRequest, ForgeBackend, NewThreadRequest, PullRequestDetails,
};
use crate::model::review::ReviewSession;
use crate::model::thread::AnchorTarget;
use crate::model::thread_store::PersistedThread;

/// The result of attempting to execute one [`PlannedOperation`].
#[derive(Debug)]
pub enum ExecutedOperation {
    /// The backend call succeeded.
    Completed(PlannedOperation),
    /// The operation's outcome was not [`OperationOutcome::Planned`]/
    /// [`OperationOutcome::Emulated`] (e.g. `Unsupported`/`Stale`/
    /// `Conflict`/`Invalid`), so it was never attempted — exactly mirroring
    /// what the dry-run preview already told the caller would happen.
    Skipped(PlannedOperation),
}

/// The full outcome of one [`execute_plan`] call.
#[derive(Debug, Default)]
pub struct PublishReport {
    /// Operations that were attempted and either completed or skipped, in
    /// plan order, up to (and including) the point of any failure.
    pub results: Vec<ExecutedOperation>,
    /// The first operation whose backend call returned an error, and that
    /// error's message. `None` if every attempted operation succeeded.
    pub failed: Option<(PlannedOperation, String)>,
    /// Operations after the failure that were never attempted at all.
    /// Empty when `failed` is `None` (every operation was attempted).
    pub remaining: Vec<PlannedOperation>,
    /// Response from the review-level operation, when one was executed.
    pub review_response: Option<crate::forge::traits::GhCreateReviewResponse>,
}

impl PublishReport {
    pub fn is_success(&self) -> bool {
        self.failed.is_none()
    }

    /// Operations that actually executed a backend call and succeeded.
    pub fn completed(&self) -> impl Iterator<Item = &PlannedOperation> {
        self.results.iter().filter_map(|r| match r {
            ExecutedOperation::Completed(op) => Some(op),
            ExecutedOperation::Skipped(_) => None,
        })
    }
}

/// Execute every operation in `plan` against `backend`, mutating `session`'s
/// threads in place as each operation succeeds. `submit_body` is the
/// review-level summary text used only for the plan's (at most one)
/// `SubmitReview` operation — every per-thread comment was already posted
/// individually via `CreateThread`/`Reply`. The standard entry point carries
/// no batched comments; the checkpointed TUI variant may include a narrowly
/// selected legacy fallback when a grouped durable root is outside the active
/// commit selection.
///
/// Stops at the first operation whose backend call returns `Err`, leaving
/// every already-completed operation's state durably persisted in
/// `session` and every not-yet-attempted operation listed in the returned
/// report's `remaining` for the caller to retry later (see this module's
/// doc comment for why simply re-planning and re-calling `execute_plan` is
/// sufficient to resume safely).
pub fn execute_plan(
    backend: &dyn ForgeBackend,
    pr: &PullRequestDetails,
    session: &mut ReviewSession,
    plan: &crate::forge::dryrun::DryRunPlan,
    submit_body: &str,
) -> PublishReport {
    execute_plan_with_checkpoint_and_comments(
        backend,
        pr,
        session,
        plan,
        submit_body,
        &[],
        |_, _| Ok(()),
    )
}

/// Execute `plan` and checkpoint the updated session after every successful
/// operation before advancing to the next provider mutation.
pub fn execute_plan_with_checkpoint(
    backend: &dyn ForgeBackend,
    pr: &PullRequestDetails,
    session: &mut ReviewSession,
    plan: &crate::forge::dryrun::DryRunPlan,
    submit_body: &str,
    checkpoint: impl FnMut(&ReviewSession, &PlannedOperation) -> Result<()>,
) -> PublishReport {
    execute_plan_with_checkpoint_and_comments(
        backend,
        pr,
        session,
        plan,
        submit_body,
        &[],
        checkpoint,
    )
}

/// Checkpointed execution variant that carries selected legacy inline
/// fallbacks on the review-level operation.
pub fn execute_plan_with_checkpoint_and_comments(
    backend: &dyn ForgeBackend,
    pr: &PullRequestDetails,
    session: &mut ReviewSession,
    plan: &crate::forge::dryrun::DryRunPlan,
    submit_body: &str,
    submit_comments: &[crate::forge::submit::InlineComment],
    mut checkpoint: impl FnMut(&ReviewSession, &PlannedOperation) -> Result<()>,
) -> PublishReport {
    let provider = plan.provider.provider_key();
    let mut report = PublishReport::default();

    for (index, op) in plan.operations.iter().enumerate() {
        if !matches!(
            op.outcome,
            OperationOutcome::Planned | OperationOutcome::Emulated { .. }
        ) {
            report.results.push(ExecutedOperation::Skipped(op.clone()));
            continue;
        }

        match execute_one(
            backend,
            pr,
            session,
            provider,
            op,
            submit_body,
            submit_comments,
        ) {
            Ok(response) => {
                if response.is_some() {
                    report.review_response = response;
                }
                report
                    .results
                    .push(ExecutedOperation::Completed(op.clone()));
                if let Err(error) = checkpoint(session, op) {
                    report.failed = Some((op.clone(), error.to_string()));
                    report.remaining = plan.operations[index + 1..].to_vec();
                    return report;
                }
            }
            Err(err) => {
                report.failed = Some((op.clone(), err.to_string()));
                report.remaining = plan.operations[index + 1..].to_vec();
                return report;
            }
        }
    }

    report
}

fn execute_one(
    backend: &dyn ForgeBackend,
    pr: &PullRequestDetails,
    session: &mut ReviewSession,
    provider: &str,
    op: &PlannedOperation,
    submit_body: &str,
    submit_comments: &[crate::forge::submit::InlineComment],
) -> Result<Option<crate::forge::traits::GhCreateReviewResponse>> {
    match &op.op {
        OperationKind::CreateThread => {
            let thread_id = op
                .thread_id
                .as_deref()
                .ok_or_else(|| missing_thread_id("CreateThread"))?;
            let persisted = find_thread_mut(session, thread_id)?;
            let request = new_thread_request(persisted, pr.head_sha.as_str())?;
            let response = backend.create_thread(pr, request)?;
            persisted.upsert_provider_mapping(provider, response.mapping);
            persisted.record_root_comment_id(provider, response.root_comment_id);
            Ok(None)
        }
        OperationKind::Reply { comment_id } => {
            let thread_id = op
                .thread_id
                .as_deref()
                .ok_or_else(|| missing_thread_id("Reply"))?;
            let persisted = find_thread_mut(session, thread_id)?;
            let body = persisted
                .thread
                .replies()
                .find(|reply| reply.id().as_str() == comment_id)
                .map(|reply| reply.body.clone())
                .ok_or_else(|| {
                    TuicrError::Forge(format!(
                        "planned reply `{comment_id}` is no longer present on thread \
                         `{thread_id}`"
                    ))
                })?;
            // Read the namespaced root-comment ledger *before* cloning the
            // bare mapping: a remote re-import between `create_thread` (or
            // the original import) and this `Reply` wholesale-replaces the
            // bare `provider` mapping (see
            // `thread_store::merge_remote_thread_into_existing`), dropping
            // any `root_comment_id` it carried, but the ledger survives
            // that replacement by design. Backends that need a
            // `root_comment_id` distinct from the bare mapping's own `id`
            // (currently: GitHub) would otherwise fail to reply to any
            // thread re-imported since it was created/first fetched.
            let root_comment_id = persisted.root_comment_id(provider).map(str::to_string);
            let mut mapping = persisted
                .provider_mapping(provider)
                .cloned()
                .ok_or_else(|| {
                    TuicrError::Forge(format!(
                        "thread `{thread_id}` has no `{provider}` mapping to reply against"
                    ))
                })?;
            if let Some(root_comment_id) = root_comment_id {
                mapping["root_comment_id"] = serde_json::Value::String(root_comment_id);
            }
            let response = backend.reply_to_thread(pr, &mapping, &body)?;
            persisted.record_published_reply(provider, comment_id.as_str(), response.comment_id);
            Ok(None)
        }
        OperationKind::Resolve | OperationKind::Reopen | OperationKind::Dismiss => {
            let thread_id = op.thread_id.as_deref().ok_or_else(|| {
                missing_thread_id(match &op.op {
                    OperationKind::Resolve => "Resolve",
                    OperationKind::Reopen => "Reopen",
                    _ => "Dismiss",
                })
            })?;
            let persisted = find_thread_mut(session, thread_id)?;
            let mapping = persisted
                .provider_mapping(provider)
                .cloned()
                .ok_or_else(|| {
                    TuicrError::Forge(format!(
                        "thread `{thread_id}` has no `{provider}` mapping to resolve/reopen"
                    ))
                })?;
            // `Dismiss` has no distinct native provider state — closing a
            // thread as "won't fix" is a local-only classification; the
            // provider only ever sees resolved/unresolved (mirrors
            // `dryrun.rs`'s `resolution_outcome` sharing one capability
            // check across `Resolve` and `Dismiss`).
            let resolved = !matches!(op.op, OperationKind::Reopen);
            let updated_mapping = backend.set_thread_resolution(pr, &mapping, resolved)?;
            persisted.upsert_provider_mapping(provider, updated_mapping);
            Ok(None)
        }
        OperationKind::SubmitReview { event } => {
            let event = parse_submit_event(event)?;
            let response = backend.create_review(
                pr,
                CreateReviewRequest {
                    event,
                    commit_id: pr.head_sha.as_str(),
                    body: submit_body,
                    comments: submit_comments,
                },
            )?;
            Ok(Some(response))
        }
    }
}

fn find_thread_mut<'a>(
    session: &'a mut ReviewSession,
    thread_id: &str,
) -> Result<&'a mut PersistedThread> {
    session
        .threads
        .iter_mut()
        .find(|thread| thread.id().as_str() == thread_id)
        .ok_or_else(|| TuicrError::Forge(format!("planned thread `{thread_id}` no longer exists")))
}

fn missing_thread_id(op_name: &str) -> TuicrError {
    TuicrError::Forge(format!(
        "planned {op_name} operation is missing its `thread_id`"
    ))
}

fn parse_submit_event(event: &str) -> Result<SubmitEvent> {
    match event {
        "comment" => Ok(SubmitEvent::Comment),
        "approve" => Ok(SubmitEvent::Approve),
        "request_changes" => Ok(SubmitEvent::RequestChanges),
        "draft" => Ok(SubmitEvent::Draft),
        other => Err(TuicrError::Forge(format!(
            "unrecognized planned SubmitReview event `{other}`"
        ))),
    }
}

/// Build the [`NewThreadRequest`] for `persisted`'s `CreateThread`
/// operation from its own anchor and root comment — never from body/path
/// heuristics reconstructed some other way, matching this thread's own
/// already-validated data.
fn new_thread_request<'a>(
    persisted: &'a PersistedThread,
    head_sha: &'a str,
) -> Result<NewThreadRequest<'a>> {
    let root = persisted.thread.root().ok_or_else(|| {
        TuicrError::Forge(format!(
            "thread `{}` has no root comment to publish",
            persisted.id().as_str()
        ))
    })?;
    let (path, line, side, range_start) = match persisted.thread.anchor().target() {
        AnchorTarget::Review => (None, None, None, None),
        AnchorTarget::File { path } => (Some(path.as_str()), None, None, None),
        AnchorTarget::Line { path, side, line } => {
            (Some(path.as_str()), Some(*line), Some(*side), None)
        }
        AnchorTarget::Range {
            path,
            side,
            start,
            end,
        } => (Some(path.as_str()), Some(*end), Some(*side), Some(*start)),
    };
    Ok(NewThreadRequest {
        commit_id: head_sha,
        body: root.body.as_str(),
        path,
        line,
        side,
        range_start,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::azure::backend::AzureDevOpsBackend;
    use crate::forge::azure::test_support::{
        MockResponse, env_mutation_lock as azure_env_lock,
        start_mock_server as start_azure_mock_server,
    };
    use crate::forge::capabilities;
    use crate::forge::dryrun::plan_publication;
    use crate::forge::giteafj::backend::GiteaForgejoBackend;
    use crate::forge::github::gh::{GhCommandError, GhCommandResult, GitHubGhBackend};
    use crate::forge::gitlab::glab::{GitLabGlabBackend, GlabCommandError, GlabCommandResult};
    use crate::forge::submit::SubmitEvent as Event;
    use crate::forge::traits::{ForgeKind, ForgeRepository};
    use crate::model::review::{ReviewSession, SessionDiffSource};
    use crate::model::thread::{
        Anchor, AnchorSide, ProviderRemap, Thread, ThreadAuthor, ThreadComment,
    };
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn pr(kind: ForgeKind) -> PullRequestDetails {
        let repository = match kind {
            ForgeKind::GitHub => ForgeRepository::github("github.com", "agavra", "tuicr"),
            ForgeKind::GitLab => ForgeRepository::gitlab("gitlab.com", "agavra", "tuicr"),
            _ => unreachable!("test only covers github/gitlab"),
        };
        PullRequestDetails {
            repository,
            number: 7,
            title: "t".to_string(),
            url: "https://example.invalid/pr/7".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            author: None,
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "head_sha_1".to_string(),
            base_sha: "base_sha_1".to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: Some("start_sha_1".to_string()),
        }
    }

    fn session_with_thread(thread: crate::model::thread_store::PersistedThread) -> ReviewSession {
        session_with_threads(vec![thread])
    }

    fn session_with_threads(
        threads: Vec<crate::model::thread_store::PersistedThread>,
    ) -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/repo"),
            "base".to_string(),
            None,
            SessionDiffSource::default(),
        );
        session.threads = threads;
        session
    }

    fn open_local_thread() -> crate::model::thread_store::PersistedThread {
        open_local_thread_at("src/a.rs", 10, "please fix this")
    }

    fn open_local_thread_at(
        path: &str,
        line: u32,
        body: &str,
    ) -> crate::model::thread_store::PersistedThread {
        let anchor = Anchor::line(path, AnchorSide::New, line);
        let root = ThreadComment::new(ThreadAuthor::human("alice"), body);
        crate::model::thread_store::PersistedThread::new(Thread::open(anchor, root))
    }

    /// A `GhCommandRunner` test double that returns responses from a
    /// pre-loaded queue, one per call, in order — sufficient for these
    /// scenario tests since call order is fully controlled by the executor
    /// and each test only needs to script exactly the calls it expects.
    #[derive(Default)]
    struct ScriptedGhRunner {
        responses: RefCell<std::collections::VecDeque<GhCommandResult<String>>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl ScriptedGhRunner {
        fn queue(self, response: &str) -> Self {
            self.responses
                .borrow_mut()
                .push_back(Ok(response.to_string()));
            self
        }

        fn queue_err(self) -> Self {
            self.responses
                .borrow_mut()
                .push_back(Err(GhCommandError::Failed {
                    status: Some(1),
                    stderr: "simulated failure".to_string(),
                }));
            self
        }
    }

    impl crate::forge::github::gh::GhCommandRunner for ScriptedGhRunner {
        fn run(&self, args: &[String]) -> GhCommandResult<String> {
            self.calls.borrow_mut().push(args.to_vec());
            self.responses.borrow_mut().pop_front().unwrap_or_else(|| {
                Err(GhCommandError::Failed {
                    status: Some(1),
                    stderr: "no scripted response left".to_string(),
                })
            })
        }

        fn run_with_stdin(&self, args: &[String], _stdin: &str) -> GhCommandResult<String> {
            self.run(args)
        }
    }

    #[derive(Default)]
    struct ScriptedGlabRunner {
        responses: RefCell<std::collections::VecDeque<GlabCommandResult<String>>>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl ScriptedGlabRunner {
        fn queue(self, response: &str) -> Self {
            self.responses
                .borrow_mut()
                .push_back(Ok(response.to_string()));
            self
        }

        fn queue_err(self) -> Self {
            self.responses
                .borrow_mut()
                .push_back(Err(GlabCommandError::Failed {
                    status: Some(1),
                    stderr: "simulated failure".to_string(),
                }));
            self
        }
    }

    impl crate::forge::gitlab::glab::GlabCommandRunner for ScriptedGlabRunner {
        fn run(&self, args: &[String]) -> GlabCommandResult<String> {
            self.calls.borrow_mut().push(args.to_vec());
            self.responses.borrow_mut().pop_front().unwrap_or_else(|| {
                Err(GlabCommandError::Failed {
                    status: Some(1),
                    stderr: "no scripted response left".to_string(),
                })
            })
        }

        fn run_with_stdin(&self, args: &[String], _stdin: &str) -> GlabCommandResult<String> {
            self.run(args)
        }
    }

    const GH_CREATE_COMMENT: &str = r#"{"id": 111, "node_id": "PRRC_1"}"#;
    const GH_THREAD_LOOKUP: &str =
        r#"{"data": {"node": {"pullRequestReviewThread": {"id": "PRRT_1", "isResolved": false}}}}"#;
    const GH_REPLY: &str = r#"{"id": 222, "node_id": "PRRC_2"}"#;
    const GH_RESOLVE: &str =
        r#"{"data": {"resolveReviewThread": {"thread": {"id": "PRRT_1", "isResolved": true}}}}"#;

    #[test]
    fn should_create_thread_then_reply_then_resolve_on_github_end_to_end() {
        let runner = ScriptedGhRunner::default()
            .queue(GH_CREATE_COMMENT)
            .queue(GH_THREAD_LOOKUP)
            .queue(GH_REPLY)
            .queue(GH_RESOLVE);
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);

        let mut thread = open_local_thread();
        let reply_id = thread
            .thread
            .reply(ThreadComment::new(ThreadAuthor::human("bob"), "on it"));
        thread.thread.resolve();
        let mut session = session_with_thread(thread);

        let plan = plan_publication(&session, &capabilities::github(), None);
        let report = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(report.is_success(), "{report:?}");
        assert_eq!(report.completed().count(), 3);

        let persisted = &session.threads[0];
        assert_eq!(
            persisted.provider_mapping("github").unwrap()["id"],
            "PRRT_1"
        );
        assert_eq!(
            persisted.provider_mapping("github").unwrap()["is_resolved"],
            true
        );
        assert_eq!(
            persisted.published_reply_id("github", reply_id.as_str()),
            Some("222")
        );
    }

    #[test]
    fn should_checkpoint_each_success_before_advancing() {
        let runner = ScriptedGhRunner::default()
            .queue(GH_CREATE_COMMENT)
            .queue(GH_THREAD_LOOKUP)
            .queue(GH_REPLY);
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);
        let mut thread = open_local_thread();
        let reply_id = thread
            .thread
            .reply(ThreadComment::new(ThreadAuthor::human("bob"), "reply"));
        let mut session = session_with_thread(thread);
        let plan = plan_publication(&session, &capabilities::github(), None);
        let mut checkpoints = Vec::new();

        let report = execute_plan_with_checkpoint(
            &backend,
            &pr,
            &mut session,
            &plan,
            "",
            |checkpointed, operation| {
                let persisted = &checkpointed.threads[0];
                match &operation.op {
                    OperationKind::CreateThread => {
                        assert!(persisted.provider_mapping("github").is_some());
                        assert!(persisted.root_comment_id("github").is_some());
                        checkpoints.push("root");
                    }
                    OperationKind::Reply { .. } => {
                        assert!(
                            persisted
                                .published_reply_id("github", reply_id.as_str())
                                .is_some()
                        );
                        checkpoints.push("reply");
                    }
                    _ => {}
                }
                Ok(())
            },
        );

        assert!(report.is_success(), "{report:?}");
        assert_eq!(checkpoints, vec!["root", "reply"]);
    }

    #[test]
    fn should_replan_without_duplicate_after_checkpoint_failure() {
        let runner = ScriptedGhRunner::default()
            .queue(GH_CREATE_COMMENT)
            .queue(GH_THREAD_LOOKUP);
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);
        let mut session = session_with_thread(open_local_thread());
        let plan = plan_publication(&session, &capabilities::github(), None);

        let report =
            execute_plan_with_checkpoint(&backend, &pr, &mut session, &plan, "", |_, _| {
                Err(TuicrError::Forge("checkpoint failed".to_string()))
            });

        assert!(!report.is_success());
        assert!(session.threads[0].provider_mapping("github").is_some());
        let retry = plan_publication(&session, &capabilities::github(), None);
        assert!(
            !retry
                .operations
                .iter()
                .any(|operation| matches!(operation.op, OperationKind::CreateThread))
        );
    }

    #[test]
    fn should_reply_using_root_comment_ledger_after_remote_reimport_replaces_bare_mapping() {
        // Regression test for a bug the parity audit found: a remote
        // re-import between `create_thread` and a later `Reply`
        // wholesale-replaces the bare `github` mapping via
        // `thread_store::merge_remote_thread_into_existing` (dropping the
        // `root_comment_id` that mapping originally carried right after
        // `create_thread`), but the namespaced `github:root` ledger
        // survives that replacement by design. `execute_one`'s `Reply` arm
        // must consult that ledger — not just the bare mapping — or this
        // scenario fails with "provider mapping is missing
        // `root_comment_id`; cannot reply" even though the correct root
        // comment ID is sitting right next to it.
        let mut thread = open_local_thread();
        let reply_id = thread
            .thread
            .reply(ThreadComment::new(ThreadAuthor::human("bob"), "on it"));
        // Simulate the post-reimport state directly: bare mapping without
        // `root_comment_id` (the exact shape `thread_from_remote` produces
        // when the remote thread's root comment carries no `rest_id`),
        // plus the ledger entry that a prior `create_thread` (or, per the
        // companion test below, an import with a `rest_id`) would have
        // left behind and which a reimport's bare-key replacement never
        // touches.
        thread.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "is_resolved": false}),
        );
        thread.record_root_comment_id("github", "111");
        assert!(
            thread
                .provider_mapping("github")
                .unwrap()
                .get("root_comment_id")
                .is_none(),
            "bare mapping must NOT carry root_comment_id in this scenario"
        );
        let mut session = session_with_thread(thread);

        let plan = plan_publication(&session, &capabilities::github(), None);
        assert!(
            !plan
                .operations
                .iter()
                .any(|op| matches!(op.op, OperationKind::CreateThread)),
            "an already-mapped thread must never replan CreateThread"
        );

        let runner = ScriptedGhRunner::default().queue(GH_REPLY);
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);
        let report = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(report.is_success(), "{report:?}");
        assert_eq!(
            session.threads[0].published_reply_id("github", reply_id.as_str()),
            Some("222"),
            "reply must succeed by falling back to the root-comment ledger"
        );
    }

    #[test]
    fn should_seed_root_comment_ledger_from_remote_import_enabling_reply_to_a_thread_never_created_locally()
     {
        // Regression test for the audit's "likely bigger practical gap":
        // a thread fetched directly from an existing remote PR (never
        // created via this tool's own `create_thread`) has to still be
        // repliable. GitHub's GraphQL thread-lookup query now requests
        // `databaseId` on each comment node (see
        // `github::review_threads::convert_comment`), surfacing a
        // REST-compatible ID as `RemoteReviewComment::rest_id`;
        // `thread_store::thread_from_remote` seeds the namespaced
        // root-comment ledger from the root comment's `rest_id` at import
        // time, the same ledger `create_thread` populates, so
        // `execute_one`'s `Reply` arm can find a root comment ID for a
        // thread it never created.
        use crate::forge::remote_comments::{
            RemoteCommentSide, RemoteReviewComment, RemoteReviewThread,
        };

        let remote = RemoteReviewThread {
            id: "PRRT_imported".to_string(),
            path: "src/a.rs".to_string(),
            line: Some(10),
            side: RemoteCommentSide::Right,
            is_resolved: false,
            is_outdated: false,
            range: None,
            provider_native_anchor: None,
            comments: vec![RemoteReviewComment {
                id: "PRRC_root_node".to_string(),
                author: Some("teammate".to_string()),
                body: "please double check this".to_string(),
                created_at: None,
                in_reply_to: None,
                url: String::new(),
                rest_id: Some("999".to_string()),
            }],
        };

        let mut session = session_with_threads(Vec::new());
        let created = session.import_remote_review_threads("github", std::slice::from_ref(&remote));
        assert_eq!(created, 1);
        assert_eq!(
            session.threads[0].root_comment_id("github"),
            Some("999"),
            "import must seed the root-comment ledger from the remote root comment's rest_id"
        );

        let reply_id = session.threads[0]
            .thread
            .reply(ThreadComment::new(ThreadAuthor::human("bob"), "on it"));

        let plan = plan_publication(&session, &capabilities::github(), None);
        assert!(
            !plan
                .operations
                .iter()
                .any(|op| matches!(op.op, OperationKind::CreateThread)),
            "an imported thread must never replan CreateThread"
        );

        let runner = ScriptedGhRunner::default().queue(GH_REPLY);
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);
        let report = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(report.is_success(), "{report:?}");
        assert_eq!(
            session.threads[0].published_reply_id("github", reply_id.as_str()),
            Some("222"),
            "reply to a never-locally-created thread must succeed"
        );
    }

    #[test]
    fn should_not_duplicate_when_the_same_plan_is_executed_twice() {
        let runner = ScriptedGhRunner::default()
            .queue(GH_CREATE_COMMENT)
            .queue(GH_THREAD_LOOKUP);
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);

        let thread = open_local_thread();
        let mut session = session_with_thread(thread);

        let plan = plan_publication(&session, &capabilities::github(), None);
        let first = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(first.is_success());
        assert_eq!(first.completed().count(), 1);

        // Re-planning now sees the thread's provider mapping and must not
        // plan `CreateThread` again; re-executing that (now-empty for this
        // thread) plan performs zero additional backend calls, proving no
        // duplicate is ever sent.
        let second_plan = plan_publication(&session, &capabilities::github(), None);
        assert!(
            !second_plan
                .operations
                .iter()
                .any(|op| matches!(op.op, OperationKind::CreateThread)),
            "must not replan CreateThread for an already-published thread"
        );
        let second = execute_plan(&backend, &pr, &mut session, &second_plan, "");
        assert!(second.is_success());
        assert_eq!(second.completed().count(), 0);
    }

    #[test]
    fn should_stop_at_first_failure_and_resume_without_resending_completed_ops() {
        // First attempt: CreateThread succeeds, the reply's resolve call
        // fails (simulated 5xx/network failure).
        let runner = ScriptedGhRunner::default()
            .queue(GH_CREATE_COMMENT)
            .queue(GH_THREAD_LOOKUP)
            .queue_err();
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);

        let mut thread = open_local_thread();
        thread.thread.resolve();
        let mut session = session_with_thread(thread);

        let plan = plan_publication(&session, &capabilities::github(), None);
        assert_eq!(plan.operations.len(), 2, "CreateThread + Resolve expected");
        let first = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(!first.is_success());
        assert_eq!(
            first.completed().count(),
            1,
            "CreateThread should have completed"
        );
        assert!(
            first.remaining.is_empty(),
            "Resolve was the last op, nothing after it"
        );

        // The thread's provider mapping from the successful CreateThread is
        // durably recorded even though Resolve failed.
        assert_eq!(
            session.threads[0].provider_mapping("github").unwrap()["id"],
            "PRRT_1"
        );

        // Resume: re-plan (CreateThread is now skipped; only Resolve is
        // still outstanding) and retry — this time it succeeds.
        let retry_runner = ScriptedGhRunner::default().queue(GH_RESOLVE);
        let retry_backend = GitHubGhBackend::with_runner(None, retry_runner);
        let resume_plan = plan_publication(&session, &capabilities::github(), None);
        assert_eq!(
            resume_plan.operations.len(),
            1,
            "only Resolve should remain planned"
        );
        assert!(matches!(
            resume_plan.operations[0].op,
            OperationKind::Resolve
        ));
        let second = execute_plan(&retry_backend, &pr, &mut session, &resume_plan, "");
        assert!(second.is_success());
        assert_eq!(
            session.threads[0].provider_mapping("github").unwrap()["is_resolved"],
            true
        );
    }

    #[test]
    fn should_create_thread_then_reply_then_resolve_on_gitlab_end_to_end() {
        let create_response = serde_json::json!({
            "id": "disc-1",
            "individual_note": false,
            "notes": [{"id": 111, "body": "please fix this", "resolved": false}],
        })
        .to_string();
        let reply_response = serde_json::json!({"id": 222, "body": "on it"}).to_string();
        let resolve_response = serde_json::json!({
            "id": "disc-1",
            "notes": [{"id": 111, "body": "please fix this", "resolved": true}],
        })
        .to_string();

        let runner = ScriptedGlabRunner::default()
            .queue(&create_response)
            .queue(&reply_response)
            .queue(&resolve_response);
        let backend = GitLabGlabBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitLab);

        let mut thread = open_local_thread();
        let reply_id = thread
            .thread
            .reply(ThreadComment::new(ThreadAuthor::human("bob"), "on it"));
        thread.thread.resolve();
        let mut session = session_with_thread(thread);

        let plan = plan_publication(&session, &capabilities::gitlab(), None);
        let report = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(report.is_success(), "{report:?}");
        assert_eq!(report.completed().count(), 3);

        let persisted = &session.threads[0];
        assert_eq!(
            persisted.provider_mapping("gitlab").unwrap()["id"],
            "disc-1"
        );
        assert_eq!(
            persisted.provider_mapping("gitlab").unwrap()["is_resolved"],
            true
        );
        assert_eq!(
            persisted.published_reply_id("gitlab", reply_id.as_str()),
            Some("222")
        );
    }

    #[test]
    fn should_preserve_lineage_and_provider_ids_across_pr_head_change_then_reply() {
        // Publish a thread against the original head, simulate a PR head
        // advance (anchor refresh with an exact provider remap — the shape
        // a real forge-driven re-anchor would supply), then add a local
        // reply and sync again. The provider mapping / root-comment lineage
        // recorded by the original `create_thread` must survive the anchor
        // refresh untouched, and the reply must be sent `in_reply_to` that
        // same original root comment rather than re-creating the thread.
        let runner = ScriptedGhRunner::default()
            .queue(GH_CREATE_COMMENT)
            .queue(GH_THREAD_LOOKUP);
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);

        let thread = open_local_thread();
        let mut session = session_with_thread(thread);

        let plan = plan_publication(&session, &capabilities::github(), None);
        let report = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(report.is_success(), "{report:?}");

        let mapping_before = session.threads[0]
            .provider_mapping("github")
            .cloned()
            .expect("thread has a github mapping after create_thread");
        assert_eq!(mapping_before["id"], "PRRT_1");
        assert_eq!(session.threads[0].root_comment_id("github"), Some("111"));

        // Simulate the PR head moving: the file's line 10 anchor is now at
        // line 25 per an exact provider remap (as a real forge adapter
        // would supply from the new diff). Content passed for context
        // relocation is irrelevant here since an exact remap always wins.
        let refresh = session.threads[0]
            .thread
            .refresh_anchor_with_remap(&["unrelated new content"], Some(&ProviderRemap::line(25)))
            .expect("exact provider remap always succeeds for a Line anchor");
        assert!(matches!(
            refresh,
            crate::model::thread::ThreadAnchorRefresh::Applied(
                crate::model::thread::AnchorRelocation::Current { new_start: 25, .. }
            )
        ));
        match session.threads[0].thread.anchor().target() {
            crate::model::thread::AnchorTarget::Line { line, .. } => assert_eq!(*line, 25),
            other => panic!("expected a Line anchor, got {other:?}"),
        }

        // Lineage/provider IDs must be untouched by the anchor refresh —
        // `refresh_anchor_with_remap` only ever mutates `Thread::anchor`,
        // never `PersistedThread`'s provider mappings.
        let mapping_after_refresh = session.threads[0]
            .provider_mapping("github")
            .cloned()
            .expect("mapping must survive a PR head change");
        assert_eq!(mapping_after_refresh, mapping_before);
        assert_eq!(
            session.threads[0].root_comment_id("github"),
            Some("111"),
            "root comment lineage must survive the anchor refresh"
        );

        // Add a local reply after the head change and sync: re-planning
        // must not replan `CreateThread` (the thread is already durably
        // mapped) and the reply must be sent against the same root
        // comment recorded before the head change.
        let reply_id = session.threads[0].thread.reply(ThreadComment::new(
            ThreadAuthor::human("bob"),
            "still applies",
        ));

        let resume_plan = plan_publication(&session, &capabilities::github(), None);
        assert!(
            !resume_plan
                .operations
                .iter()
                .any(|op| matches!(op.op, OperationKind::CreateThread)),
            "must not replan CreateThread for a thread that survived a head change"
        );
        assert!(
            resume_plan
                .operations
                .iter()
                .any(|op| matches!(&op.op, OperationKind::Reply { comment_id } if comment_id == reply_id.as_str())),
            "the new local reply must be planned"
        );

        let reply_runner = ScriptedGhRunner::default().queue(GH_REPLY);
        let reply_backend = GitHubGhBackend::with_runner(None, reply_runner);
        let resume_report = execute_plan(&reply_backend, &pr, &mut session, &resume_plan, "");
        assert!(resume_report.is_success(), "{resume_report:?}");
        assert_eq!(
            session.threads[0].published_reply_id("github", reply_id.as_str()),
            Some("222")
        );
        // The provider mapping (and its embedded root-comment lineage) is
        // still exactly what `create_thread` produced before the head
        // change — replying never touches it.
        assert_eq!(
            session.threads[0].provider_mapping("github").cloned(),
            Some(mapping_before)
        );
    }

    #[test]
    fn should_submit_review_outcome_via_legacy_create_review_with_no_batched_comments() {
        let runner = ScriptedGhRunner::default().queue(
            r#"{"id": 999, "html_url": "https://example.invalid/pr/7#pullrequestreview-999", "state": "APPROVED"}"#,
        );
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);
        let mut session = session_with_thread(open_local_thread());
        session.threads[0].upsert_provider_mapping("github", serde_json::json!({"id": "PRRT_1"}));

        let plan = plan_publication(&session, &capabilities::github(), Some(Event::Approve));
        let submit_op = plan
            .operations
            .iter()
            .find(|op| matches!(op.op, OperationKind::SubmitReview { .. }))
            .expect("submit review op present");
        assert_eq!(submit_op.outcome, OperationOutcome::Planned);

        let report = execute_plan(&backend, &pr, &mut session, &plan, "LGTM");
        assert!(report.is_success(), "{report:?}");
    }

    #[test]
    fn should_skip_operations_whose_outcome_is_not_planned_or_emulated() {
        // A file/general comment thread with a range anchor mismatch — use
        // an unsupported placement (a range comment against Azure DevOps'
        // dual-side capability requested against github's single-side-only
        // capability is still Planned, so instead force `Unsupported` via
        // the review-level submit path with no general-comment support is
        // hard to trigger without a provider; simplest reliable trigger is
        // an `Unsupported` resolve on a capabilities profile with no
        // thread-resolution mechanism, e.g. Gitea 1.24 stable.
        let runner = ScriptedGhRunner::default();
        let backend = GitHubGhBackend::with_runner(None, runner);
        // Use a Gitea-shaped PR/capabilities pairing purely to get an
        // `Unsupported` Resolve outcome without any backend call; the
        // executor must skip it rather than attempt (and fail) a call this
        // backend does not implement.
        let pr = PullRequestDetails {
            repository: ForgeRepository::gitea("gitea.example.com", "agavra", "tuicr"),
            ..pr(ForgeKind::GitHub)
        };
        let mut thread = open_local_thread();
        thread.thread.resolve();
        thread.upsert_provider_mapping("gitea", serde_json::json!({"id": "1"}));
        let mut session = session_with_thread(thread);

        let plan = plan_publication(&session, &capabilities::gitea_1_24(), None);
        let resolve_op = plan
            .operations
            .iter()
            .find(|op| matches!(op.op, OperationKind::Resolve))
            .expect("resolve op present");
        assert!(matches!(
            resolve_op.outcome,
            OperationOutcome::Unsupported { .. }
        ));

        let report = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(
            report.is_success(),
            "skipped ops are not failures: {report:?}"
        );
        assert_eq!(report.completed().count(), 0);
        assert!(
            report
                .results
                .iter()
                .any(|r| matches!(r, ExecutedOperation::Skipped(op) if matches!(op.op, OperationKind::Resolve)))
        );
    }

    #[test]
    fn should_dismiss_thread_via_the_same_resolution_call_as_resolve() {
        // Exhaustive operation-graph mapping proof (mandatory constraint
        // 5): every `OperationKind` variant must have a proven, exercised
        // executor path. `CreateThread`/`Reply`/`Resolve`/`Reopen`/
        // `SubmitReview` each already have a dedicated scenario test above;
        // this is `Dismiss`'s — it shares `Resolve`'s provider call (no
        // distinct native "won't fix" state exists on GitHub or GitLab),
        // so completing it must still durably record `is_resolved: true`.
        let runner = ScriptedGhRunner::default()
            .queue(GH_CREATE_COMMENT)
            .queue(GH_THREAD_LOOKUP)
            .queue(GH_RESOLVE);
        let backend = GitHubGhBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitHub);

        let mut thread = open_local_thread();
        thread.thread.dismiss();
        let mut session = session_with_thread(thread);

        let plan = plan_publication(&session, &capabilities::github(), None);
        assert!(
            plan.operations
                .iter()
                .any(|op| matches!(op.op, OperationKind::Dismiss)),
            "expected a planned Dismiss operation, got {plan:?}"
        );
        let report = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(report.is_success(), "{report:?}");
        assert_eq!(report.completed().count(), 2, "CreateThread + Dismiss");
        assert_eq!(
            session.threads[0].provider_mapping("github").unwrap()["is_resolved"],
            true
        );
    }

    #[test]
    fn should_resume_gitlab_one_at_a_time_prefix_failure_without_reposting_created_threads() {
        // Mandatory constraint 6: GitLab posts each thread as its own
        // `/discussions` call (no single ambiguous batch endpoint the way
        // GitHub's legacy `create_review` has) — a mid-loop failure must
        // leave an unambiguous, provider-ID-backed per-op state: threads
        // already created (a "prefix" of the plan) are durably mapped, and
        // a retry replans only the threads still missing a mapping,
        // resuming exactly at the failure point without re-posting the
        // already-created prefix.
        let thread_a_created = serde_json::json!({
            "id": "disc-a",
            "individual_note": false,
            "notes": [{"id": 1, "body": "fix a", "resolved": false}],
        })
        .to_string();

        let runner = ScriptedGlabRunner::default()
            .queue(&thread_a_created) // thread A's CreateThread succeeds
            .queue_err(); // thread B's CreateThread fails (simulated 5xx)
        let backend = GitLabGlabBackend::with_runner(None, runner);
        let pr = pr(ForgeKind::GitLab);

        let thread_a = open_local_thread_at("src/a.rs", 10, "fix a");
        let thread_b = open_local_thread_at("src/b.rs", 20, "fix b");
        let mut session = session_with_threads(vec![thread_a, thread_b]);

        let plan = plan_publication(&session, &capabilities::gitlab(), None);
        assert_eq!(plan.operations.len(), 2, "two independent CreateThreads");
        let first = execute_plan(&backend, &pr, &mut session, &plan, "");
        assert!(!first.is_success());
        assert_eq!(
            first.completed().count(),
            1,
            "thread A's CreateThread should have completed before thread B's failed"
        );

        // Thread A's provider mapping is durably recorded — a naive retry
        // must not re-post it as a duplicate discussion.
        assert_eq!(
            session.threads[0].provider_mapping("gitlab").unwrap()["id"],
            "disc-a"
        );
        assert!(session.threads[1].provider_mapping("gitlab").is_none());

        // Resume: re-plan against the partially-updated session (thread A
        // is now skipped; only thread B's CreateThread remains) and retry.
        let thread_b_created = serde_json::json!({
            "id": "disc-b",
            "individual_note": false,
            "notes": [{"id": 2, "body": "fix b", "resolved": false}],
        })
        .to_string();
        let retry_runner = ScriptedGlabRunner::default().queue(&thread_b_created);
        let retry_backend = GitLabGlabBackend::with_runner(None, retry_runner);
        let resume_plan = plan_publication(&session, &capabilities::gitlab(), None);
        assert_eq!(
            resume_plan.operations.len(),
            1,
            "only thread B's CreateThread should remain planned"
        );
        assert_eq!(
            resume_plan.operations[0].thread_id.as_deref(),
            Some(session.threads[1].id().as_str())
        );
        let second = execute_plan(&retry_backend, &pr, &mut session, &resume_plan, "");
        assert!(second.is_success(), "{second:?}");
        assert_eq!(
            session.threads[1].provider_mapping("gitlab").unwrap()["id"],
            "disc-b"
        );
        // Thread A's mapping from the first attempt is untouched — never
        // resent.
        assert_eq!(
            session.threads[0].provider_mapping("gitlab").unwrap()["id"],
            "disc-a"
        );
    }

    /// Run `body` with `$AZURE_DEVOPS_EXT_PAT` set to a mock value **and**
    /// `$PATH` cleared, restoring both afterward — mirrors
    /// `azure::contract_tests::with_mock_pat`'s pattern (private to its own
    /// module, so this module needs its own copy), guarded by the same
    /// crate-wide `azure_env_lock` so it never races that module's own
    /// tests mutating the same process-wide env vars. Without clearing
    /// `$PATH` this would risk shelling out to a real, locally-configured
    /// `az` CLI — forbidden by this task's "no live external provider
    /// writes/calls" constraint.
    fn with_mock_azure_pat<T>(body: impl FnOnce() -> T) -> T {
        let _guard = azure_env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous_pat = std::env::var("AZURE_DEVOPS_EXT_PAT").ok();
        let previous_path = std::env::var("PATH").ok();
        // SAFETY: serialized by `azure_env_lock` above, so no other test
        // thread observes a partial/torn value while this one mutates
        // these process-wide env vars.
        unsafe {
            std::env::set_var("AZURE_DEVOPS_EXT_PAT", "mock-pat");
            std::env::remove_var("PATH");
        }
        let result = body();
        unsafe {
            match previous_pat {
                Some(v) => std::env::set_var("AZURE_DEVOPS_EXT_PAT", v),
                None => std::env::remove_var("AZURE_DEVOPS_EXT_PAT"),
            }
            match previous_path {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
        result
    }

    fn azure_pr(base_url: &str) -> PullRequestDetails {
        PullRequestDetails {
            repository: ForgeRepository::azure_devops(base_url, "contoso/widgets", "api"),
            number: 7,
            title: "t".to_string(),
            url: "https://example.invalid/pr/7".to_string(),
            state: "active".to_string(),
            is_draft: false,
            author: None,
            head_ref_name: "feature".to_string(),
            base_ref_name: "main".to_string(),
            head_sha: "head_sha_1".to_string(),
            base_sha: "base_sha_1".to_string(),
            body: String::new(),
            updated_at: None,
            closed: false,
            merged_at: None,
            diff_start_sha: Some("start_sha_1".to_string()),
        }
    }

    /// End-to-end proof (audit finding: "execute_plan mock tests for Azure
    /// ... every Planned op either executes through the backend once or
    /// profile marks Unsupported") that Azure's `CreateThread`/`Reply`/
    /// `Resolve` — all three planned `Native`/`Planned` on
    /// `capabilities::azure_devops()` — each execute through exactly one
    /// real (mocked) transport call, mirroring the existing GitHub/GitLab
    /// scenario tests above.
    #[test]
    fn should_create_thread_then_reply_then_resolve_on_azure_end_to_end() {
        with_mock_azure_pat(|| {
            let base = "/contoso/widgets/_apis/git/repositories/api/pullrequests/7";
            let responses = HashMap::from([
                (
                    format!("POST {base}/threads?api-version=7.1"),
                    MockResponse::json(
                        200,
                        r#"{"id":501,"status":"active","comments":[{"id":9001}]}"#,
                    ),
                ),
                (
                    format!("POST {base}/threads/501/comments?api-version=7.1"),
                    MockResponse::json(200, r#"{"id":9002}"#),
                ),
                (
                    format!("PATCH {base}/threads/501?api-version=7.1"),
                    MockResponse::json(200, r#"{"id":501,"status":"fixed"}"#),
                ),
            ]);
            let (base_url, requests) = start_azure_mock_server(responses);
            let backend = AzureDevOpsBackend::new(None);
            let pr = azure_pr(&base_url);

            let mut thread = open_local_thread();
            let reply_id = thread
                .thread
                .reply(ThreadComment::new(ThreadAuthor::human("bob"), "on it"));
            thread.thread.resolve();
            let mut session = session_with_thread(thread);

            let plan = plan_publication(&session, &capabilities::azure_devops(), None);
            let report = execute_plan(&backend, &pr, &mut session, &plan, "");
            assert!(report.is_success(), "{report:?}");
            assert_eq!(
                report.completed().count(),
                3,
                "CreateThread + Reply + Resolve"
            );

            let persisted = &session.threads[0];
            assert_eq!(
                persisted.provider_mapping("azure-devops").unwrap()["id"],
                "501"
            );
            assert_eq!(
                persisted.provider_mapping("azure-devops").unwrap()["is_resolved"],
                true
            );
            assert_eq!(
                persisted.published_reply_id("azure-devops", reply_id.as_str()),
                Some("9002")
            );
            assert_eq!(
                requests.lock().expect("lock captured requests").len(),
                3,
                "exactly one real call per planned operation, no more, no fewer"
            );
        });
    }

    /// End-to-end proof (audit finding: "no planned->default Unsupported
    /// mismatch") for the Gitea/Forgejo `CreateThread` honesty fix this
    /// offline-integration pass made in `dryrun::plan_thread`: a brand-new
    /// thread's `CreateThread` op is planned `Unsupported` (see
    /// `capabilities::CreateThreadSupport`), and `execute_plan` correctly
    /// `Skip`s it rather than attempting (and inevitably failing, or
    /// worse, silently reaching for) a real HTTP call this backend never
    /// implements — proving the dry-run preview and the real executor
    /// agree with zero live requests made.
    #[test]
    fn should_skip_create_thread_as_unsupported_on_gitea_and_forgejo_end_to_end() {
        let cases: [(
            ForgeKind,
            capabilities::ProviderCapabilities,
            ForgeRepository,
        ); 2] = [
            (
                ForgeKind::Gitea,
                capabilities::gitea_1_24(),
                ForgeRepository::gitea("http://127.0.0.1:1", "owner", "repo"),
            ),
            (
                ForgeKind::Forgejo,
                capabilities::forgejo_16(),
                ForgeRepository::forgejo("http://127.0.0.1:1", "owner", "repo"),
            ),
        ];
        for (kind, caps, repo) in cases {
            // No mock server is even started: if `execute_plan` ever tried
            // a real call here, it would fail to connect (nothing is
            // listening on this address) rather than silently succeed,
            // making any accidental live-call regression loud, not silent.
            let backend = GiteaForgejoBackend::new(kind, None);
            let pr = PullRequestDetails {
                repository: repo,
                number: 7,
                title: "t".to_string(),
                url: "https://example.invalid/pr/7".to_string(),
                state: "open".to_string(),
                is_draft: false,
                author: None,
                head_ref_name: "feature".to_string(),
                base_ref_name: "main".to_string(),
                head_sha: "head_sha_1".to_string(),
                base_sha: "base_sha_1".to_string(),
                body: String::new(),
                updated_at: None,
                closed: false,
                merged_at: None,
                diff_start_sha: Some("start_sha_1".to_string()),
            };

            let thread = open_local_thread();
            let mut session = session_with_thread(thread);

            let plan = plan_publication(&session, &caps, None);
            assert_eq!(plan.operations.len(), 1, "kind={kind:?}, plan={plan:?}");
            assert!(
                matches!(
                    plan.operations[0].outcome,
                    OperationOutcome::Unsupported { .. }
                ),
                "kind={kind:?}, outcome={:?}",
                plan.operations[0].outcome
            );

            let report = execute_plan(&backend, &pr, &mut session, &plan, "");
            assert!(report.is_success(), "kind={kind:?}, {report:?}");
            assert_eq!(report.completed().count(), 0, "kind={kind:?}");
            assert!(
                matches!(&report.results[0], ExecutedOperation::Skipped(_)),
                "kind={kind:?}, results={:?}",
                report.results
            );
            assert!(
                session.threads[0]
                    .provider_mapping(kind.provider_key())
                    .is_none(),
                "kind={kind:?}: a skipped CreateThread must never record a provider mapping"
            );
        }
    }
}
