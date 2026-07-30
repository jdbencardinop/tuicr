//! Pure dry-run publication planner.
//!
//! `plan_publication` walks a [`ReviewSession`]'s durable threads (and an
//! optional intended review-level outcome) against a
//! [`ProviderCapabilities`] profile and returns an exact per-operation
//! [`DryRunPlan`]. It performs no I/O, needs no credentials, and never
//! contacts a provider — it is safe to call for any of the five profiles in
//! [`crate::forge::capabilities`], including the three that have no real
//! transport yet.
//!
//! Every comment, reply, and resolution/dismissal change in the session
//! produces exactly one [`PlannedOperation`] with one of exactly six
//! [`OperationOutcome`]s (`planned`, `unsupported`, `emulated`, `stale`,
//! `conflict`, `invalid`) per
//! `docs/decisions/review-artifact-contract.md`'s "Operations return one
//! of..." section. The planner never silently skips a thread, reply, range,
//! or intended outcome — an operation it cannot execute natively is always
//! reported as `unsupported`/`emulated`/`stale`/`conflict`/`invalid`, never
//! omitted from `operations`.
//!
//! Publication idempotency/dedup (e.g. not re-creating an already-published
//! thread) is the eventual `review sync`/`review publish` command's
//! responsibility, not this planner's: every thread and comment in the
//! session is always represented by an operation here, regardless of any
//! existing `provider_mappings` entry, so nothing is ever silently dropped
//! from the plan.

use serde::{Deserialize, Serialize};

use crate::forge::capabilities::{
    ProviderCapabilities, RangeSupport, ReplySupport, RequestChangesSupport, ThreadResolutionLevel,
};
use crate::forge::submit::SubmitEvent;
use crate::forge::traits::ForgeKind;
use crate::model::review::ReviewSession;
use crate::model::thread::{AnchorSide, AnchorState, AnchorTarget};
use crate::model::thread_store::PersistedThread;

/// One planned remote operation and its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedOperation {
    /// The thread this operation targets, or `None` for the single
    /// review-level submit operation (see [`OperationKind::SubmitReview`]).
    pub thread_id: Option<String>,
    #[serde(flatten)]
    pub op: OperationKind,
    #[serde(flatten)]
    pub outcome: OperationOutcome,
}

/// What kind of remote call an operation represents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum OperationKind {
    /// Create the thread's root comment.
    CreateThread,
    /// Post one reply (a non-root comment already present in the thread).
    Reply { comment_id: String },
    /// Mark the thread resolved.
    Resolve,
    /// Reopen a previously resolved thread.
    Reopen,
    /// Mark the thread dismissed ("won't fix").
    Dismiss,
    /// Submit the review-level outcome (`comment`/`approve`/
    /// `request_changes`/`draft`).
    SubmitReview { event: String },
}

/// The result of attempting one operation, per
/// `docs/decisions/review-artifact-contract.md`. Exactly one of these six
/// shapes — nothing is ever silently coerced into `planned`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum OperationOutcome {
    /// Will execute with native provider semantics.
    Planned,
    /// The provider has no mechanism for this operation and no substitute is
    /// modeled.
    Unsupported { reason: String },
    /// The provider has no native mechanism, but this exact substitute
    /// behavior will be used instead — never silent.
    Emulated { substitute: String },
    /// The anchor has no unique relocation in the current diff (zero context
    /// matches).
    Stale { detail: String },
    /// The anchor has more than one possible relocation in the current diff
    /// (ambiguous placement).
    Conflict { detail: String },
    /// The underlying data is structurally invalid (e.g. corrupted/
    /// hand-edited session JSON bypassing the normal validating
    /// constructors) and cannot be planned at all.
    Invalid { reason: String },
}

/// The full dry-run plan for one publication attempt against one provider
/// profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunPlan {
    /// Serialized via [`ForgeKind::provider_key`] (`"github"`, `"gitea"`,
    /// ...) rather than this type's derived `Serialize` (which would emit
    /// `"git_hub"`), so the dry-run JSON's provider value round-trips with
    /// the CLI's own `--provider` flag values.
    #[serde(with = "provider_key_serde")]
    pub provider: ForgeKind,
    pub provider_version: Option<String>,
    pub operations: Vec<PlannedOperation>,
}

mod provider_key_serde {
    use super::ForgeKind;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(kind: &ForgeKind, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(kind.provider_key())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ForgeKind, D::Error> {
        let key = String::deserialize(deserializer)?;
        ForgeKind::from_provider_key(&key)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown provider key {key:?}")))
    }
}

/// Plan every thread/comment/resolution operation in `session`, plus at most
/// one review-level submit operation when `intended_outcome` is supplied,
/// against `capabilities`.
pub fn plan_publication(
    session: &ReviewSession,
    capabilities: &ProviderCapabilities,
    intended_outcome: Option<SubmitEvent>,
) -> DryRunPlan {
    let mut operations = Vec::new();

    for persisted in session.threads() {
        plan_thread(persisted, capabilities, &mut operations);
    }

    if let Some(event) = intended_outcome {
        operations.push(PlannedOperation {
            thread_id: None,
            op: OperationKind::SubmitReview {
                event: submit_event_key(event).to_string(),
            },
            outcome: submit_review_outcome(event, capabilities),
        });
    }

    DryRunPlan {
        provider: capabilities.kind,
        provider_version: capabilities.version.as_ref().map(|v| v.0.clone()),
        operations,
    }
}

fn submit_event_key(event: SubmitEvent) -> &'static str {
    match event {
        SubmitEvent::Comment => "comment",
        SubmitEvent::Approve => "approve",
        SubmitEvent::RequestChanges => "request_changes",
        SubmitEvent::Draft => "draft",
    }
}

fn submit_review_outcome(
    event: SubmitEvent,
    capabilities: &ProviderCapabilities,
) -> OperationOutcome {
    match event {
        SubmitEvent::Comment => {
            if capabilities.general_comment {
                OperationOutcome::Planned
            } else {
                OperationOutcome::Unsupported {
                    reason: "provider has no verified general/review-level comment mechanism"
                        .to_string(),
                }
            }
        }
        // Every profiled provider has some approve mechanism (native review
        // event, approval endpoint, or a positive vote) per
        // provider-semantics.md's review-lifecycle table; none require
        // emulation or are unsupported.
        SubmitEvent::Approve => OperationOutcome::Planned,
        SubmitEvent::RequestChanges => match &capabilities.request_changes {
            RequestChangesSupport::Native | RequestChangesSupport::Vote => {
                OperationOutcome::Planned
            }
            RequestChangesSupport::Emulated { substitute } => OperationOutcome::Emulated {
                substitute: substitute.clone(),
            },
            RequestChangesSupport::Unsupported => OperationOutcome::Unsupported {
                reason: "provider has no request-changes mechanism, native or emulated".to_string(),
            },
        },
        SubmitEvent::Draft => {
            if capabilities.pending_review.supported {
                OperationOutcome::Planned
            } else {
                OperationOutcome::Unsupported {
                    reason: "provider has no pending/draft review object; a draft cannot be kept \
                             private without one"
                        .to_string(),
                }
            }
        }
    }
}

fn plan_thread(
    persisted: &PersistedThread,
    capabilities: &ProviderCapabilities,
    operations: &mut Vec<PlannedOperation>,
) {
    let thread_id = persisted.id().as_str().to_string();
    let thread = &persisted.thread;

    let anchor_outcome = anchor_outcome(
        thread.anchor().target(),
        thread.anchor().state(),
        capabilities,
    );

    operations.push(PlannedOperation {
        thread_id: Some(thread_id.clone()),
        op: OperationKind::CreateThread,
        outcome: anchor_outcome.clone(),
    });

    for reply in thread.replies() {
        let outcome = if !matches!(anchor_outcome, OperationOutcome::Planned) {
            // A reply to a comment that can't itself be placed inherits the
            // same placement problem — never silently claim a reply would
            // succeed when its parent comment could not be created.
            anchor_outcome.clone()
        } else {
            match &capabilities.reply {
                ReplySupport::Native => OperationOutcome::Planned,
                ReplySupport::Emulated { substitute } => OperationOutcome::Emulated {
                    substitute: substitute.clone(),
                },
                ReplySupport::Unsupported => OperationOutcome::Unsupported {
                    reason: "provider has no verified reply mechanism for this host/version"
                        .to_string(),
                },
            }
        };
        operations.push(PlannedOperation {
            thread_id: Some(thread_id.clone()),
            op: OperationKind::Reply {
                comment_id: reply.id().as_str().to_string(),
            },
            outcome,
        });
    }

    let resolution_outcome = || {
        // A thread whose own anchor can't be placed can't be resolved/
        // dismissed either — inherit the same stale/conflict/invalid/
        // unsupported outcome instead of independently claiming the
        // resolution would succeed (mirrors the reply-inheritance rule
        // above).
        if !matches!(anchor_outcome, OperationOutcome::Planned) {
            return anchor_outcome.clone();
        }
        match capabilities.thread_resolution {
            ThreadResolutionLevel::None => OperationOutcome::Unsupported {
                reason: "provider has no verified thread-resolution mechanism for this \
                         host/version"
                    .to_string(),
            },
            ThreadResolutionLevel::Comment
            | ThreadResolutionLevel::Discussion
            | ThreadResolutionLevel::Thread => OperationOutcome::Planned,
        }
    };

    match thread.status() {
        crate::model::thread::ThreadStatus::Resolved => operations.push(PlannedOperation {
            thread_id: Some(thread_id.clone()),
            op: OperationKind::Resolve,
            outcome: resolution_outcome(),
        }),
        crate::model::thread::ThreadStatus::Dismissed => operations.push(PlannedOperation {
            thread_id: Some(thread_id),
            op: OperationKind::Dismiss,
            outcome: resolution_outcome(),
        }),
        _ => {}
    }
}

/// Determine the outcome of anchoring (creating/replying to) `target` given
/// its current relocation `state` and `capabilities`. Shared by
/// `CreateThread` and every `Reply` under it.
fn anchor_outcome(
    target: &AnchorTarget,
    state: AnchorState,
    capabilities: &ProviderCapabilities,
) -> OperationOutcome {
    match state {
        AnchorState::Stale => {
            return OperationOutcome::Stale {
                detail: "anchor context has zero matches in the current diff".to_string(),
            };
        }
        AnchorState::Ambiguous => {
            return OperationOutcome::Conflict {
                detail: "anchor context matches more than one place in the current diff"
                    .to_string(),
            };
        }
        AnchorState::Current => {}
    }

    match target {
        AnchorTarget::Review => {
            if capabilities.general_comment {
                OperationOutcome::Planned
            } else {
                OperationOutcome::Unsupported {
                    reason: "provider has no verified review-level general comment mechanism"
                        .to_string(),
                }
            }
        }
        AnchorTarget::File { .. } => {
            if capabilities.file_comment {
                OperationOutcome::Planned
            } else {
                OperationOutcome::Unsupported {
                    reason: "provider has no verified whole-file comment mechanism".to_string(),
                }
            }
        }
        AnchorTarget::Line { side, .. } => side_outcome(*side, capabilities),
        AnchorTarget::Range {
            side, start, end, ..
        } => {
            if end < start {
                return OperationOutcome::Invalid {
                    reason: format!(
                        "range anchor end {end} is before start {start}; session data is \
                         corrupted or was hand-edited outside the validating constructors"
                    ),
                };
            }
            if let OperationOutcome::Unsupported { reason } = side_outcome(*side, capabilities) {
                return OperationOutcome::Unsupported { reason };
            }
            match capabilities.range {
                RangeSupport::None => OperationOutcome::Emulated {
                    substitute: format!(
                        "post as a single-line comment anchored at line {start}; the provider \
                         does not support multi-line ranges and would otherwise silently drop \
                         the rest of the range (lines {start}-{end})"
                    ),
                },
                RangeSupport::SameSide => {
                    if *side == AnchorSide::Both {
                        OperationOutcome::Unsupported {
                            reason: "provider supports a range on one diff side only, not a \
                                     single range spanning both old and new sides"
                                .to_string(),
                        }
                    } else {
                        OperationOutcome::Planned
                    }
                }
                RangeSupport::DualSideOffsets => OperationOutcome::Planned,
            }
        }
    }
}

fn side_outcome(side: AnchorSide, capabilities: &ProviderCapabilities) -> OperationOutcome {
    let supported = match side {
        AnchorSide::Old => capabilities.sides.old,
        AnchorSide::New => capabilities.sides.new,
        AnchorSide::Both => capabilities.sides.simultaneous,
    };
    if supported {
        OperationOutcome::Planned
    } else {
        OperationOutcome::Unsupported {
            reason: format!("provider cannot anchor a comment to diff side {side:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forge::capabilities::{azure_devops, forgejo_16, gitea_1_24, github, gitlab};
    use crate::model::review::{ReviewSession, SessionDiffSource};
    use crate::model::thread::{Anchor, Thread, ThreadAuthor, ThreadComment};
    use std::path::PathBuf;

    fn session_with_threads(threads: Vec<PersistedThread>) -> ReviewSession {
        let mut session = ReviewSession::new(
            PathBuf::from("/repo"),
            "deadbeef".to_string(),
            None,
            SessionDiffSource::WorkingTree,
        );
        session.threads = threads;
        session
    }

    fn open_thread(anchor: Anchor) -> PersistedThread {
        let root = ThreadComment::new(ThreadAuthor::human("alice"), "root comment");
        PersistedThread::new(Thread::open(anchor, root))
    }

    fn find_op<'a>(plan: &'a DryRunPlan, thread_id: &str) -> &'a PlannedOperation {
        plan.operations
            .iter()
            .find(|op| op.thread_id.as_deref() == Some(thread_id))
            .expect("operation for thread")
    }

    #[test]
    fn should_plan_line_comment_thread_as_planned_on_every_profile() {
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let thread = open_thread(anchor);
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        for caps in [
            github(),
            gitlab(),
            azure_devops(),
            gitea_1_24(),
            forgejo_16(),
        ] {
            let plan = plan_publication(&session, &caps, None);
            let op = find_op(&plan, &thread_id);
            assert_eq!(op.outcome, OperationOutcome::Planned, "caps={caps:?}");
        }
    }

    #[test]
    fn should_emulate_range_comment_as_single_line_on_gitea_but_plan_on_forgejo() {
        let anchor = Anchor::range("src/a.rs", AnchorSide::New, 10, 12).unwrap();
        let thread = open_thread(anchor);
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let gitea_plan = plan_publication(&session, &gitea_1_24(), None);
        match &find_op(&gitea_plan, &thread_id).outcome {
            OperationOutcome::Emulated { substitute } => {
                assert!(substitute.contains("single-line"));
            }
            other => panic!("expected Emulated on gitea, got {other:?}"),
        }

        let forgejo_plan = plan_publication(&session, &forgejo_16(), None);
        assert_eq!(
            find_op(&forgejo_plan, &thread_id).outcome,
            OperationOutcome::Planned,
            "forgejo 16 live-accepts extra_lines_count range comments"
        );
    }

    #[test]
    fn should_plan_dual_side_range_on_azure_but_not_on_github() {
        // Simulate a range anchor that needs both sides at once by directly
        // checking side capability via a Both-side line anchor, since the
        // domain `Anchor` constructors don't expose a "Both" range helper.
        let anchor = Anchor::line("src/a.rs", AnchorSide::Both, 10);
        let thread = open_thread(anchor);
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let azure_plan = plan_publication(&session, &azure_devops(), None);
        assert_eq!(
            find_op(&azure_plan, &thread_id).outcome,
            OperationOutcome::Planned
        );

        let github_plan = plan_publication(&session, &github(), None);
        match &find_op(&github_plan, &thread_id).outcome {
            OperationOutcome::Unsupported { .. } => {}
            other => panic!("expected Unsupported on github, got {other:?}"),
        }
    }

    #[test]
    fn should_mark_stale_anchor_as_stale_outcome() {
        let anchor = Anchor::line_with_context(
            "src/a.rs",
            AnchorSide::New,
            10,
            crate::model::AnchorContext {
                before: vec![],
                selected: vec!["original content".to_string()],
                after: vec![],
            },
        )
        .unwrap();
        let mut thread = open_thread(anchor);
        thread
            .thread
            .refresh_anchor_with_remap(&["completely", "different", "file"], None)
            .unwrap();
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        match &find_op(&plan, &thread_id).outcome {
            OperationOutcome::Stale { .. } => {}
            other => panic!("expected Stale, got {other:?}"),
        }
    }

    #[test]
    fn should_mark_ambiguous_anchor_as_conflict_outcome() {
        // A single-line file whose only line matches "dup" twice makes any
        // 1-line selection ambiguous once context is captured against a
        // 2-line file with the same content on both lines.
        let anchor = Anchor::line_with_context(
            "src/a.rs",
            AnchorSide::New,
            1,
            crate::model::AnchorContext {
                before: vec![],
                selected: vec!["dup".to_string()],
                after: vec![],
            },
        )
        .unwrap();
        let mut thread = open_thread(anchor);
        thread
            .thread
            .refresh_anchor_with_remap(&["dup", "dup"], None)
            .unwrap();
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        match &find_op(&plan, &thread_id).outcome {
            OperationOutcome::Conflict { .. } => {}
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn should_propagate_stale_placement_to_replies() {
        let anchor = Anchor::line_with_context(
            "src/a.rs",
            AnchorSide::New,
            10,
            crate::model::AnchorContext {
                before: vec![],
                selected: vec!["original content".to_string()],
                after: vec![],
            },
        )
        .unwrap();
        let mut thread = open_thread(anchor);
        thread
            .thread
            .refresh_anchor_with_remap(&["nope"], None)
            .unwrap();
        thread
            .thread
            .reply(ThreadComment::new(ThreadAuthor::human("bob"), "reply"));
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        let reply_op = plan
            .operations
            .iter()
            .find(|op| matches!(op.op, OperationKind::Reply { .. }))
            .expect("reply operation present");
        assert_eq!(
            reply_op.outcome,
            OperationOutcome::Stale {
                detail: "anchor context has zero matches in the current diff".to_string()
            }
        );
        assert_eq!(reply_op.thread_id.as_deref(), Some(thread_id.as_str()));
    }

    #[test]
    fn should_plan_reply_native_on_github_but_unsupported_on_gitea_and_forgejo_stable() {
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let mut thread = open_thread(anchor);
        thread
            .thread
            .reply(ThreadComment::new(ThreadAuthor::human("bob"), "reply"));
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let github_plan = plan_publication(&session, &github(), None);
        let github_reply = github_plan
            .operations
            .iter()
            .find(|op| matches!(op.op, OperationKind::Reply { .. }))
            .unwrap();
        assert_eq!(github_reply.outcome, OperationOutcome::Planned);

        for caps in [gitea_1_24(), forgejo_16()] {
            let plan = plan_publication(&session, &caps, None);
            let reply_op = plan
                .operations
                .iter()
                .find(|op| matches!(op.op, OperationKind::Reply { .. }))
                .unwrap();
            match &reply_op.outcome {
                OperationOutcome::Unsupported { .. } => {}
                other => panic!("expected Unsupported on {caps:?}, got {other:?}"),
            }
            assert_eq!(reply_op.thread_id.as_deref(), Some(thread_id.as_str()));
        }
    }

    #[test]
    fn should_plan_resolve_as_unsupported_on_gitea_and_forgejo_stable_but_planned_on_github() {
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let mut thread = open_thread(anchor);
        thread.thread.resolve();
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let github_plan = plan_publication(&session, &github(), None);
        let resolve_op = github_plan
            .operations
            .iter()
            .find(|op| matches!(op.op, OperationKind::Resolve))
            .expect("resolve op present");
        assert_eq!(resolve_op.outcome, OperationOutcome::Planned);

        for caps in [gitea_1_24(), forgejo_16()] {
            let plan = plan_publication(&session, &caps, None);
            let resolve_op = plan
                .operations
                .iter()
                .find(|op| matches!(op.op, OperationKind::Resolve))
                .unwrap();
            match &resolve_op.outcome {
                OperationOutcome::Unsupported { .. } => {}
                other => panic!("expected Unsupported on {caps:?}, got {other:?}"),
            }
            let _ = &thread_id;
        }
    }

    #[test]
    fn should_plan_dismiss_operation_when_thread_is_dismissed() {
        let anchor = Anchor::review();
        let mut thread = open_thread(anchor);
        thread.thread.dismiss();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        assert!(
            plan.operations
                .iter()
                .any(|op| matches!(op.op, OperationKind::Dismiss))
        );
    }

    #[test]
    fn should_inherit_stale_anchor_outcome_for_resolve_and_dismiss_not_planned() {
        // A resolved/dismissed thread whose anchor is stale can't be
        // created/located in the first place, so github()'s native
        // thread-resolution support must not override that with a false
        // `Planned` resolve/dismiss outcome.
        let stale_anchor = || {
            let anchor = Anchor::line_with_context(
                "src/a.rs",
                AnchorSide::New,
                10,
                crate::model::AnchorContext {
                    before: vec![],
                    selected: vec!["original content".to_string()],
                    after: vec![],
                },
            )
            .unwrap();
            let mut thread = open_thread(anchor);
            thread
                .thread
                .refresh_anchor_with_remap(&["completely", "different", "file"], None)
                .unwrap();
            thread
        };

        let mut resolved = stale_anchor();
        resolved.thread.resolve();
        let resolved_id = resolved.id().as_str().to_string();

        let mut dismissed = stale_anchor();
        dismissed.thread.dismiss();
        let dismissed_id = dismissed.id().as_str().to_string();

        let session = session_with_threads(vec![resolved, dismissed]);
        let plan = plan_publication(&session, &github(), None);

        let resolve_op = plan
            .operations
            .iter()
            .find(|op| {
                op.thread_id.as_deref() == Some(resolved_id.as_str())
                    && matches!(op.op, OperationKind::Resolve)
            })
            .expect("resolve op present");
        match &resolve_op.outcome {
            OperationOutcome::Stale { .. } => {}
            other => panic!("expected Stale, got {other:?}"),
        }

        let dismiss_op = plan
            .operations
            .iter()
            .find(|op| {
                op.thread_id.as_deref() == Some(dismissed_id.as_str())
                    && matches!(op.op, OperationKind::Dismiss)
            })
            .expect("dismiss op present");
        match &dismiss_op.outcome {
            OperationOutcome::Stale { .. } => {}
            other => panic!("expected Stale, got {other:?}"),
        }
    }

    #[test]
    fn should_inherit_conflict_anchor_outcome_for_resolve_and_dismiss_not_planned() {
        // Same rule as the stale case above, but for an ambiguous
        // (multi-match) anchor that resolves to `Conflict`.
        let ambiguous_anchor = || {
            let anchor = Anchor::line_with_context(
                "src/a.rs",
                AnchorSide::New,
                1,
                crate::model::AnchorContext {
                    before: vec![],
                    selected: vec!["dup".to_string()],
                    after: vec![],
                },
            )
            .unwrap();
            let mut thread = open_thread(anchor);
            thread
                .thread
                .refresh_anchor_with_remap(&["dup", "dup"], None)
                .unwrap();
            thread
        };

        let mut resolved = ambiguous_anchor();
        resolved.thread.resolve();
        let resolved_id = resolved.id().as_str().to_string();

        let mut dismissed = ambiguous_anchor();
        dismissed.thread.dismiss();
        let dismissed_id = dismissed.id().as_str().to_string();

        let session = session_with_threads(vec![resolved, dismissed]);
        let plan = plan_publication(&session, &github(), None);

        let resolve_op = plan
            .operations
            .iter()
            .find(|op| {
                op.thread_id.as_deref() == Some(resolved_id.as_str())
                    && matches!(op.op, OperationKind::Resolve)
            })
            .expect("resolve op present");
        match &resolve_op.outcome {
            OperationOutcome::Conflict { .. } => {}
            other => panic!("expected Conflict, got {other:?}"),
        }

        let dismiss_op = plan
            .operations
            .iter()
            .find(|op| {
                op.thread_id.as_deref() == Some(dismissed_id.as_str())
                    && matches!(op.op, OperationKind::Dismiss)
            })
            .expect("dismiss op present");
        match &dismiss_op.outcome {
            OperationOutcome::Conflict { .. } => {}
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn should_inherit_invalid_anchor_outcome_for_resolve_and_dismiss_not_planned() {
        // Same rule again, but for a structurally corrupted (`Invalid`)
        // anchor loaded from a hand-edited/corrupted session file.
        let invalid_anchor = || {
            let anchor_json = serde_json::json!({
                "target": {
                    "kind": "range",
                    "path": "src/a.rs",
                    "side": "new",
                    "start": 10,
                    "end": 5
                },
                "state": "current",
                "context": null
            });
            let anchor: Anchor = serde_json::from_value(anchor_json).unwrap();
            open_thread(anchor)
        };

        let mut resolved = invalid_anchor();
        resolved.thread.resolve();
        let resolved_id = resolved.id().as_str().to_string();

        let mut dismissed = invalid_anchor();
        dismissed.thread.dismiss();
        let dismissed_id = dismissed.id().as_str().to_string();

        let session = session_with_threads(vec![resolved, dismissed]);
        let plan = plan_publication(&session, &github(), None);

        let resolve_op = plan
            .operations
            .iter()
            .find(|op| {
                op.thread_id.as_deref() == Some(resolved_id.as_str())
                    && matches!(op.op, OperationKind::Resolve)
            })
            .expect("resolve op present");
        match &resolve_op.outcome {
            OperationOutcome::Invalid { .. } => {}
            other => panic!("expected Invalid, got {other:?}"),
        }

        let dismiss_op = plan
            .operations
            .iter()
            .find(|op| {
                op.thread_id.as_deref() == Some(dismissed_id.as_str())
                    && matches!(op.op, OperationKind::Dismiss)
            })
            .expect("dismiss op present");
        match &dismiss_op.outcome {
            OperationOutcome::Invalid { .. } => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn should_not_plan_resolve_or_dismiss_for_an_open_thread() {
        let anchor = Anchor::review();
        let thread = open_thread(anchor);
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        assert!(
            !plan
                .operations
                .iter()
                .any(|op| matches!(op.op, OperationKind::Resolve | OperationKind::Dismiss))
        );
    }

    #[test]
    fn should_reject_corrupted_range_anchor_as_invalid_not_silently_planned() {
        // Bypass the validating `Anchor::range` constructor entirely via a
        // raw JSON round-trip, the same technique `thread_store.rs` uses
        // internally, to simulate a hand-edited/corrupted session file
        // whose `end` is before `start`.
        let anchor_json = serde_json::json!({
            "target": {
                "kind": "range",
                "path": "src/a.rs",
                "side": "new",
                "start": 10,
                "end": 5
            },
            "state": "current",
            "context": null
        });
        let anchor: Anchor = serde_json::from_value(anchor_json).unwrap();
        let thread = open_thread(anchor);
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        match &find_op(&plan, &thread_id).outcome {
            OperationOutcome::Invalid { .. } => {}
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn should_plan_submit_review_comment_and_approve() {
        let session = session_with_threads(vec![]);
        let caps = github();

        let comment_plan = plan_publication(&session, &caps, Some(SubmitEvent::Comment));
        assert_eq!(comment_plan.operations.len(), 1);
        assert_eq!(
            comment_plan.operations[0].outcome,
            OperationOutcome::Planned
        );

        let approve_plan = plan_publication(&session, &caps, Some(SubmitEvent::Approve));
        assert_eq!(
            approve_plan.operations[0].outcome,
            OperationOutcome::Planned
        );
    }

    #[test]
    fn should_emulate_gitlab_request_changes_as_unresolved_discussion() {
        let session = session_with_threads(vec![]);
        let plan = plan_publication(&session, &gitlab(), Some(SubmitEvent::RequestChanges));
        match &plan.operations[0].outcome {
            OperationOutcome::Emulated { substitute } => {
                assert!(substitute.contains("unresolved"));
            }
            other => panic!("expected Emulated, got {other:?}"),
        }
    }

    #[test]
    fn should_plan_vote_based_request_changes_on_azure_devops() {
        let session = session_with_threads(vec![]);
        let plan = plan_publication(&session, &azure_devops(), Some(SubmitEvent::RequestChanges));
        assert_eq!(plan.operations[0].outcome, OperationOutcome::Planned);
    }

    #[test]
    fn should_mark_draft_unsupported_when_no_pending_review_object() {
        let session = session_with_threads(vec![]);
        let plan = plan_publication(&session, &gitlab(), Some(SubmitEvent::Draft));
        match &plan.operations[0].outcome {
            OperationOutcome::Unsupported { .. } => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
    }

    #[test]
    fn should_include_provider_kind_and_version_in_plan() {
        let session = session_with_threads(vec![]);
        let plan = plan_publication(&session, &forgejo_16(), None);
        assert_eq!(plan.provider, ForgeKind::Forgejo);
        assert_eq!(plan.provider_version.as_deref(), Some("16.0.1"));
    }

    #[test]
    fn should_round_trip_plan_json() {
        let anchor = Anchor::range("src/a.rs", AnchorSide::New, 1, 3).unwrap();
        let thread = open_thread(anchor);
        let session = session_with_threads(vec![thread]);
        let plan = plan_publication(&session, &gitea_1_24(), Some(SubmitEvent::RequestChanges));

        let json = serde_json::to_string_pretty(&plan).unwrap();
        let restored: DryRunPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(plan, restored);
    }
}
