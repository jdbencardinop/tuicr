//! Pure dry-run publication planner.
//!
//! `plan_publication` walks a [`ReviewSession`]'s durable threads (and an
//! optional intended review-level outcome) against a
//! [`ProviderCapabilities`] profile and returns an exact per-operation
//! [`DryRunPlan`]. It performs no I/O, needs no credentials, and never
//! contacts a provider — it is safe to call for any of the five profiles in
//! [`crate::forge::capabilities`], and its outcome always matches what
//! `crate::forge::publish::execute_plan` will actually do against that
//! provider's real transport (see `crate::forge::traits::ForgeBackend`'s
//! `create_thread`/`reply_to_thread`/`set_thread_resolution`).
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
//! Publication idempotency/dedup is this planner's responsibility, not a
//! deferred one: a thread already carrying a `provider_mappings` entry for
//! `capabilities.kind` (i.e. already published or imported from that
//! provider) never gets a `CreateThread` operation planned again, and any
//! comment whose author is [`AuthorKind::Remote`] (it was fetched *from*
//! the provider, e.g. via
//! [`crate::model::review::ReviewSession::import_remote_review_threads`])
//! never gets a `Reply` operation planned, since posting either would
//! create a duplicate on the provider. Only genuinely new, not-yet-published
//! content — a brand-new thread with no provider mapping, or a
//! locally-authored (`Human`/`Agent`) reply added after import — is ever
//! planned as `CreateThread`/`Reply`. `Resolve` is similarly skipped once
//! the thread's own `provider_mappings` snapshot already reports
//! `is_resolved: true` for this provider, so re-running the planner after a
//! successful resolve does not replan it. Nothing already-published is
//! ever silently *represented* as still-pending, and nothing new is ever
//! silently dropped: outside these known-already-done cases, an operation
//! this planner cannot execute natively is always reported as
//! `unsupported`/`emulated`/`stale`/`conflict`/`invalid` instead of an
//! entry it merely omits from `operations`, per
//! `docs/decisions/review-artifact-contract.md`'s "Operations return one
//! of..." section.

use serde::{Deserialize, Serialize};

use crate::forge::capabilities::{
    CreateThreadSupport, ProviderCapabilities, RangeSupport, ReplySupport, RequestChangesSupport,
    ThreadResolutionLevel,
};
use crate::forge::submit::SubmitEvent;
use crate::forge::traits::ForgeKind;
use crate::model::review::ReviewSession;
use crate::model::thread::AuthorKind;
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
    let provider_mapping = persisted.provider_mapping(capabilities.kind.provider_key());
    // A thread already carries this provider's mapping once it has been
    // published *or* imported from it (see `thread_from_remote` /
    // `import_remote_review_threads`, which always calls
    // `upsert_provider_mapping` on the provider it imported from) — either
    // way, its root comment already exists there, so planning another
    // `CreateThread` would post a duplicate.
    let already_published = provider_mapping.is_some();

    let anchor_outcome = anchor_outcome(
        thread.anchor().target(),
        thread.anchor().state(),
        capabilities,
    );

    if !already_published {
        // Structural anchor problems (the anchor itself has no unique/any
        // valid placement in the current diff) always take priority, same
        // as everywhere else in this planner — they are a property of the
        // session data, not of the provider. Only once the anchor itself
        // is fine does whether this provider has any verified standalone
        // create-thread endpoint at all become the deciding factor: a
        // profile with `CreateThreadSupport::Unsupported` (currently
        // Gitea/Forgejo — see that variant's doc comment) can never plan
        // `CreateThread` as `Planned`/`Emulated`, no matter how the anchor
        // itself would otherwise be handled, since `ForgeBackend::
        // create_thread`'s honest default (`UnsupportedOperation`) is
        // never overridden for such a profile.
        let create_outcome = match &anchor_outcome {
            OperationOutcome::Stale { .. }
            | OperationOutcome::Conflict { .. }
            | OperationOutcome::Invalid { .. } => anchor_outcome.clone(),
            _ if matches!(capabilities.create_thread, CreateThreadSupport::Unsupported) => {
                OperationOutcome::Unsupported {
                    reason: format!(
                        "{} has no verified standalone create-thread endpoint on this \
                         profile (only a batched pending-review comment route is evidenced)",
                        capabilities.kind.provider_key()
                    ),
                }
            }
            _ => anchor_outcome.clone(),
        };
        operations.push(PlannedOperation {
            thread_id: Some(thread_id.clone()),
            op: OperationKind::CreateThread,
            outcome: create_outcome,
        });
    }

    for reply in thread.replies() {
        // A reply fetched *from* the provider (`AuthorKind::Remote`, see
        // `thread_from_remote`) already exists there verbatim — planning it
        // again would post the same content a second time. Only replies
        // authored locally (`Human`/`Agent`) after that fetch are new,
        // not-yet-published content.
        if reply.author.kind == AuthorKind::Remote {
            continue;
        }
        // A locally authored reply that was already published in a prior
        // `execute_plan` run is recorded in the `"{provider}:replies"`
        // ledger (see `PersistedThread::record_published_reply`), which is
        // namespaced separately from the bare provider mapping so it
        // survives `merge_remote_thread_into_existing`'s wholesale
        // replacement of that key on the next remote re-import. Without
        // this check, re-running the planner after a successful publish
        // (but before any remote re-fetch marks the reply `Remote`) would
        // plan — and `execute_plan` would resend — the same reply again.
        if persisted
            .published_reply_id(capabilities.kind.provider_key(), reply.id().as_str())
            .is_some()
        {
            continue;
        }
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

    // The imported/published snapshot's own `is_resolved` flag (stored by
    // `thread_from_remote` at import time) tells us whether the provider
    // already reflects this thread's current `Resolved` status — if so,
    // re-running the planner (e.g. after a successful sync) must not
    // replan the same `Resolve` call. Dismiss has no equivalent
    // provider-native flag captured on import, so it is always planned
    // while the thread is `Dismissed` (repeating a resolve/close call is
    // the provider's own idempotency concern, not a duplicate-content risk
    // like `CreateThread`/`Reply`).
    let already_resolved_upstream = provider_mapping
        .and_then(|mapping| mapping.get("is_resolved"))
        .and_then(|value| value.as_bool())
        == Some(true);

    match thread.status() {
        crate::model::thread::ThreadStatus::Resolved if !already_resolved_upstream => operations
            .push(PlannedOperation {
                thread_id: Some(thread_id.clone()),
                op: OperationKind::Resolve,
                outcome: resolution_outcome(),
            }),
        // The local thread has been reopened (or was never resolved
        // locally) but the provider's last-known snapshot still shows it
        // resolved — plan a `Reopen` so the two sides converge. Without
        // this arm `Reopen` was defined but never constructed, silently
        // leaving already-resolved-upstream threads that get locally
        // reopened stuck out of sync.
        crate::model::thread::ThreadStatus::Open if already_resolved_upstream => {
            operations.push(PlannedOperation {
                thread_id: Some(thread_id.clone()),
                op: OperationKind::Reopen,
                outcome: resolution_outcome(),
            })
        }
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
    fn should_plan_line_comment_thread_as_planned_where_create_thread_is_natively_supported() {
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let thread = open_thread(anchor);
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        // GitHub, GitLab, and Azure DevOps each verified a dedicated
        // standalone create-thread endpoint (`CreateThreadSupport::Native`).
        for caps in [github(), gitlab(), azure_devops()] {
            let plan = plan_publication(&session, &caps, None);
            let op = find_op(&plan, &thread_id);
            assert_eq!(op.outcome, OperationOutcome::Planned, "caps={caps:?}");
        }
    }

    /// Regression test for the offline-integration merge audit's "no
    /// planned->default Unsupported mismatch" requirement: Gitea/Forgejo
    /// only verified a *batched* pending-review comment route
    /// (`create_review`), never a standalone single-comment endpoint, so
    /// `ForgeBackend::create_thread`'s honest default
    /// (`UnsupportedOperation`) is never overridden for either — meaning
    /// `plan_publication` must plan `CreateThread` as `Unsupported` for
    /// both, never `Planned`/`Emulated`, even though their `file_comment`/
    /// `general_comment` flags are `true` (those describe the batched
    /// route, a separate axis — see `CreateThreadSupport`'s doc comment).
    #[test]
    fn should_plan_create_thread_as_unsupported_on_gitea_and_forgejo_stable() {
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let thread = open_thread(anchor);
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        for caps in [gitea_1_24(), forgejo_16()] {
            let plan = plan_publication(&session, &caps, None);
            let op = find_op(&plan, &thread_id);
            match &op.outcome {
                OperationOutcome::Unsupported { reason } => {
                    assert!(
                        reason.contains("create-thread"),
                        "reason should name the missing capability, got {reason:?}"
                    );
                }
                other => panic!("expected Unsupported on {caps:?}, got {other:?}"),
            }
        }
    }

    /// Gitea and Forgejo diverge on range-anchor handling (Gitea silently
    /// drops `extra_lines_count`, so it must be modeled as an `Emulated`
    /// single-line substitute; Forgejo natively accepts it) — a real,
    /// evidenced difference between the two stable pins that is
    /// independent of, and unaffected by, `CreateThreadSupport` (which
    /// currently gates *both* to `Unsupported` before this distinction is
    /// ever reached for `CreateThread` specifically; it remains directly
    /// tested here, at the `anchor_outcome` level, so it stays proven
    /// correct for whenever a future ticket adds a verified Gitea/Forgejo
    /// create-thread route and lifts that gate).
    #[test]
    fn should_emulate_range_comment_as_single_line_on_gitea_but_plan_on_forgejo() {
        let target = AnchorTarget::Range {
            path: "src/a.rs".to_string(),
            side: AnchorSide::New,
            start: 10,
            end: 12,
        };

        match anchor_outcome(&target, AnchorState::Current, &gitea_1_24()) {
            OperationOutcome::Emulated { substitute } => {
                assert!(substitute.contains("single-line"));
            }
            other => panic!("expected Emulated on gitea, got {other:?}"),
        }

        assert_eq!(
            anchor_outcome(&target, AnchorState::Current, &forgejo_16()),
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
    fn should_not_plan_reopen_when_thread_is_open_and_provider_never_reported_resolved() {
        // Sanity check: a plain open thread with no prior provider mapping
        // (or a mapping that never reported `is_resolved: true`) must not
        // get a stray `Reopen` planned alongside the `CreateThread`.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let thread = open_thread(anchor);
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        assert!(
            !plan
                .operations
                .iter()
                .any(|op| matches!(op.op, OperationKind::Reopen))
        );
    }

    #[test]
    fn should_plan_reopen_when_locally_reopened_thread_was_resolved_upstream() {
        // The provider's last-known snapshot (captured at import/publish
        // time in the bare provider mapping's `is_resolved` flag) says the
        // thread is resolved, but the local thread has since been reopened
        // (`ThreadStatus::Open`). The two sides have diverged, so a
        // `Reopen` must be planned to converge them — this exercises the
        // previously-dead `OperationKind::Reopen` arm.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let mut thread = open_thread(anchor);
        thread.thread.resolve();
        thread.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "is_resolved": true}),
        );
        thread.thread.reopen();
        assert_eq!(
            thread.thread.status(),
            crate::model::thread::ThreadStatus::Open
        );
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        let reopen_op = plan
            .operations
            .iter()
            .find(|op| {
                op.thread_id.as_deref() == Some(thread_id.as_str())
                    && matches!(op.op, OperationKind::Reopen)
            })
            .expect("reopen op present");
        assert_eq!(reopen_op.outcome, OperationOutcome::Planned);
        assert!(
            !plan.operations.iter().any(|op| {
                op.thread_id.as_deref() == Some(thread_id.as_str())
                    && matches!(op.op, OperationKind::Resolve)
            }),
            "must not also plan Resolve for the same reopened thread"
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
    fn should_emulate_gitlab_request_changes_via_reviewer_state_graphql_mutation() {
        let session = session_with_threads(vec![]);
        let plan = plan_publication(&session, &gitlab(), Some(SubmitEvent::RequestChanges));
        match &plan.operations[0].outcome {
            OperationOutcome::Emulated { substitute } => {
                // Truthful per the parity audit (§3/§8): dry-run's message
                // must describe the real `mergeRequestRequestChanges`
                // GraphQL mutation `glab.rs`'s `create_review` actually
                // sends, not a fictional "leave an unresolved discussion"
                // substitute.
                assert!(substitute.contains("mergeRequestRequestChanges"));
                assert!(!substitute.contains("open an unresolved discussion"));
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

    #[test]
    fn should_not_plan_create_thread_for_thread_already_mapped_to_this_provider() {
        // A thread carrying a `provider_mappings` entry for github (e.g.
        // from a prior successful publish, or from
        // `import_remote_review_threads`) already exists there — planning
        // `CreateThread` again would post a duplicate root comment.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let mut thread = open_thread(anchor);
        thread.upsert_provider_mapping("github", serde_json::json!({"id": "PRRT_1"}));
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        assert!(
            !plan.operations.iter().any(|op| {
                op.thread_id.as_deref() == Some(thread_id.as_str())
                    && matches!(op.op, OperationKind::CreateThread)
            }),
            "must not replan CreateThread for an already-mapped thread"
        );
    }

    #[test]
    fn should_plan_create_thread_for_a_brand_new_unmapped_thread() {
        // Sanity check for the negative test above: a thread with no
        // provider mapping at all is genuinely new content and must still
        // be planned normally.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let thread = open_thread(anchor);
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        let create_op = find_op(&plan, &thread_id);
        assert!(matches!(create_op.op, OperationKind::CreateThread));
        assert_eq!(create_op.outcome, OperationOutcome::Planned);
    }

    #[test]
    fn should_not_plan_reply_for_a_remote_authored_reply_but_should_for_a_local_one() {
        // A reply fetched from the provider (`AuthorKind::Remote`) already
        // exists there; a reply added locally afterwards is genuinely new
        // and must still be planned.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let mut thread = open_thread(anchor);
        thread.upsert_provider_mapping("github", serde_json::json!({"id": "PRRT_1"}));
        thread.thread.reply(ThreadComment::new(
            ThreadAuthor::remote("bob", "IC_remote_1"),
            "already on github",
        ));
        thread.thread.reply(ThreadComment::new(
            ThreadAuthor::human("alice"),
            "new local reply",
        ));
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        let reply_ops: Vec<_> = plan
            .operations
            .iter()
            .filter(|op| {
                op.thread_id.as_deref() == Some(thread_id.as_str())
                    && matches!(op.op, OperationKind::Reply { .. })
            })
            .collect();
        assert_eq!(
            reply_ops.len(),
            1,
            "only the locally-authored reply should be planned, got {reply_ops:?}"
        );
        assert_eq!(reply_ops[0].outcome, OperationOutcome::Planned);
    }

    #[test]
    fn should_not_replan_a_locally_authored_reply_already_recorded_in_the_publish_ledger() {
        // A reply authored locally (`Human`/`Agent`) and successfully
        // published by a prior `execute_plan` run is recorded in the
        // `"{provider}:replies"` ledger via `record_published_reply`
        // (independent of whether the thread has since been re-imported/
        // re-fetched and thus never gets `AuthorKind::Remote` retroactively
        // applied to it). Re-planning must recognise the ledger entry and
        // skip it — otherwise every dry-run/execute cycle after the first
        // successful publish would resend the same reply.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let mut thread = open_thread(anchor);
        thread.upsert_provider_mapping("github", serde_json::json!({"id": "PRRT_1"}));
        let reply_id = thread.thread.reply(ThreadComment::new(
            ThreadAuthor::human("alice"),
            "already published in a prior run",
        ));
        thread.record_published_reply("github", reply_id.as_str(), "PRRC_999");
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        assert!(
            !plan.operations.iter().any(|op| {
                op.thread_id.as_deref() == Some(thread_id.as_str())
                    && matches!(op.op, OperationKind::Reply { .. })
            }),
            "must not replan a reply already recorded in the publish ledger"
        );
    }

    #[test]
    fn should_not_plan_resolve_when_provider_mapping_already_reports_resolved() {
        // Re-running the planner after a successful resolve/sync (or after
        // importing an already-resolved remote thread) must not replan the
        // same `Resolve` call.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let mut thread = open_thread(anchor);
        thread.thread.resolve();
        thread.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "is_resolved": true}),
        );
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        assert!(
            !plan.operations.iter().any(|op| {
                op.thread_id.as_deref() == Some(thread_id.as_str())
                    && matches!(op.op, OperationKind::Resolve)
            }),
            "must not replan Resolve once provider mapping already reports is_resolved"
        );
    }

    #[test]
    fn should_plan_resolve_when_provider_mapping_reports_not_yet_resolved() {
        // Sanity check for the negative test above: a mapped-but-not-yet-
        // resolved-upstream thread must still get a Resolve planned.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let mut thread = open_thread(anchor);
        thread.thread.resolve();
        thread.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "is_resolved": false}),
        );
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        let resolve_op = plan
            .operations
            .iter()
            .find(|op| {
                op.thread_id.as_deref() == Some(thread_id.as_str())
                    && matches!(op.op, OperationKind::Resolve)
            })
            .expect("resolve op present");
        assert_eq!(resolve_op.outcome, OperationOutcome::Planned);
    }

    #[test]
    fn should_produce_a_no_op_plan_for_a_fully_imported_unchanged_thread() {
        // The end-to-end double-publication scenario: a thread imported
        // wholesale from a remote provider (root + reply both
        // `AuthorKind::Remote`, mapping present, already resolved upstream)
        // must produce zero operations for that thread when re-planned,
        // proving re-running `review publish --dry-run` after an import
        // does not attempt to recreate any of its already-published
        // content.
        let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
        let root = ThreadComment::new(ThreadAuthor::remote("alice", "IC_root"), "root");
        let mut thread = PersistedThread::new(Thread::open(anchor, root));
        thread.thread.reply(ThreadComment::new(
            ThreadAuthor::remote("bob", "IC_reply"),
            "reply",
        ));
        thread.thread.resolve();
        thread.upsert_provider_mapping(
            "github",
            serde_json::json!({"id": "PRRT_1", "is_resolved": true}),
        );
        let thread_id = thread.id().as_str().to_string();
        let session = session_with_threads(vec![thread]);

        let plan = plan_publication(&session, &github(), None);
        assert!(
            !plan
                .operations
                .iter()
                .any(|op| op.thread_id.as_deref() == Some(thread_id.as_str())),
            "a fully-imported, unchanged thread must plan zero operations, got {:?}",
            plan.operations
        );
    }
}
