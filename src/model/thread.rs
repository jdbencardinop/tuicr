//! Provider-neutral review thread contract.
//!
//! This module freezes the thread/anchor behavior described in the
//! companion research repository's `docs/decisions/review-artifact-contract.md`
//! and `schemas/review-artifact-v1.schema.json` ahead of any `ReviewStore`
//! migration. Types here are intentionally standalone: nothing in
//! `ReviewSession`, `ReviewStore`, the TUI, or the forge adapters depends on
//! them yet. They exist to give the next thread-store implementation a
//! small, already-tested domain vocabulary to build against.
//!
//! Covered by design:
//! - stable thread IDs and immutable comment/reply IDs;
//! - human/agent (and remote-imported) authorship;
//! - an open/resolved thread lifecycle with explicit reopen limited to
//!   `Resolved` threads (`Dismissed` is terminal: it means "won't fix" and
//!   can never be reopened or resolved again); both closed statuses
//!   (`Resolved`/`Dismissed`) freeze anchor relocation and target changes —
//!   `Resolved` until reopened, `Dismissed` permanently. An explicit
//!   provider-outdated signal may still monotonically mark a current anchor
//!   stale without moving its target. Reopening
//!   re-derives status from the anchor's frozen `state` (`Current` =>
//!   `Open`, `Stale` => `Stale`, `Ambiguous` => `Ambiguous`) instead of
//!   always forcing `Open`, so a thread that was already stale or
//!   ambiguous when resolved does not come back with a false `Open`;
//! - review/file/line/range anchors, with old/new/both sides for line and
//!   range anchors;
//! - current/stale/ambiguous anchor states. Resolution priority is: (1) an
//!   exact [`ProviderRemap`] supplied by a provider adapter for an imported
//!   remote thread always wins outright, otherwise (2) unique
//!   surrounding-context relocation is used — never nearest-line guessing.
//!   `ProviderRemap` is a small explicit input type only; this module has no
//!   knowledge of any real provider adapter, keeping it domain/test-only.
//! - anchor context is validated at construction (ordering, bounds, and
//!   selected-length-matches-span) instead of trusting caller-supplied
//!   indices, and relocation derives the new span from the matched content
//!   rather than from the anchor's own prior bookkeeping;
//! - lifecycle-critical fields (stable IDs, `Thread::status`/`anchor`/
//!   `comments`, `Anchor::state`/`target`, `ThreadComment`'s id) are
//!   private with read-only accessors, so every mutation — including
//!   comment append — is routed through a tested method (`reply`, for
//!   comments) rather than a direct field assignment or public `Vec`
//!   access.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{Result, TuicrError};

/// A stable identifier for a thread. Stable across anchor relocation and
/// staleness/ambiguity per the contract's identity rules: "A thread ID is
/// stable while its anchor relocates or becomes stale."
///
/// The inner value is private: a `ThreadId` can only be minted via
/// [`ThreadId::new`] (or restored via `Deserialize`), never freely
/// constructed from an arbitrary string, so nothing outside this module can
/// forge or corrupt a thread's identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThreadId(String);

impl ThreadId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ThreadId {
    fn default() -> Self {
        Self::new()
    }
}

/// An immutable identifier for a single comment or reply. Per the contract:
/// "A comment/reply ID is immutable." The inner value is private for the
/// same reason as [`ThreadId`]: only [`CommentId::new`] (or `Deserialize`)
/// can produce one.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommentId(String);

impl CommentId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for CommentId {
    fn default() -> Self {
        Self::new()
    }
}

/// Who authored a comment or reply. Mirrors
/// `schemas/review-artifact-v1.schema.json#/$defs/author.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    /// A person using the tool directly.
    Human,
    /// An automated agent acting on a human's behalf.
    Agent,
    /// Imported from a remote provider thread whose original author kind
    /// (human or bot) is not distinguished by the provider payload.
    Remote,
}

/// The author of a [`ThreadComment`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadAuthor {
    pub kind: AuthorKind,
    pub name: String,
    pub provider_id: Option<String>,
}

impl ThreadAuthor {
    pub fn human(name: impl Into<String>) -> Self {
        Self {
            kind: AuthorKind::Human,
            name: name.into(),
            provider_id: None,
        }
    }

    pub fn agent(name: impl Into<String>) -> Self {
        Self {
            kind: AuthorKind::Agent,
            name: name.into(),
            provider_id: None,
        }
    }

    pub fn remote(name: impl Into<String>, provider_id: impl Into<String>) -> Self {
        Self {
            kind: AuthorKind::Remote,
            name: name.into(),
            provider_id: Some(provider_id.into()),
        }
    }

    pub fn is_human(&self) -> bool {
        self.kind == AuthorKind::Human
    }

    pub fn is_agent(&self) -> bool {
        self.kind == AuthorKind::Agent
    }
}

/// One comment or reply within a [`Thread`]. The first comment added to a
/// thread is its root; every comment after that is a reply. `id` never
/// changes once assigned; it is private, with a read-only [`Self::id`]
/// accessor, so nothing can reassign it after construction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadComment {
    id: CommentId,
    pub author: ThreadAuthor,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl ThreadComment {
    pub fn new(author: ThreadAuthor, body: impl Into<String>) -> Self {
        Self {
            id: CommentId::new(),
            author,
            body: body.into(),
            created_at: Utc::now(),
            updated_at: None,
        }
    }

    pub fn id(&self) -> &CommentId {
        &self.id
    }
}

/// Which side(s) of a diff an anchor targets. Mirrors
/// `schemas/review-artifact-v1.schema.json#/$defs/anchor.side`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSide {
    Old,
    New,
    Both,
}

/// What a thread is anchored to: the whole review, a whole file, a single
/// line on one side of a diff, or an inclusive line range on one side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnchorTarget {
    Review,
    File {
        path: String,
    },
    Line {
        path: String,
        side: AnchorSide,
        line: u32,
    },
    Range {
        path: String,
        side: AnchorSide,
        start: u32,
        end: u32,
    },
}

impl AnchorTarget {
    pub fn path(&self) -> Option<&str> {
        match self {
            AnchorTarget::Review => None,
            AnchorTarget::File { path }
            | AnchorTarget::Line { path, .. }
            | AnchorTarget::Range { path, .. } => Some(path.as_str()),
        }
    }
}

/// Resolution state of an anchor after the underlying diff/head changes.
/// Mirrors `schemas/review-artifact-v1.schema.json#/$defs/anchor.state`.
///
/// Per the contract: an exact provider remap (see [`ProviderRemap`]) always
/// wins over context relocation when supplied to
/// [`Anchor::relocate_with_remap`], regardless of what context relocation
/// alone would have concluded. Without a provider remap, unique
/// surrounding-context relocation is used: zero context matches is always
/// `Stale`; more than one match is always `Ambiguous`. Anchors never
/// silently move to the nearest line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorState {
    Current,
    Stale,
    Ambiguous,
}

/// Bounded before/after line context captured at anchor time. Used to
/// relocate a line/range anchor by finding the unique place in an updated
/// file where the selected lines still appear with the same surrounding
/// context.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AnchorContext {
    pub before: Vec<String>,
    pub selected: Vec<String>,
    pub after: Vec<String>,
}

impl AnchorContext {
    /// Capture context around the 0-based inclusive `[start_idx, end_idx]`
    /// selection in `lines`, taking up to `window` lines before and after.
    ///
    /// Returns [`TuicrError::InvalidInput`] instead of panicking when
    /// `start_idx` is after `end_idx`, or when `end_idx` is out of bounds
    /// for `lines`.
    pub fn capture(
        lines: &[&str],
        start_idx: usize,
        end_idx: usize,
        window: usize,
    ) -> Result<Self> {
        if start_idx > end_idx {
            return Err(TuicrError::InvalidInput(format!(
                "anchor context start_idx {start_idx} is after end_idx {end_idx}"
            )));
        }
        if end_idx >= lines.len() {
            return Err(TuicrError::InvalidInput(format!(
                "anchor context end_idx {end_idx} is out of bounds for {} lines",
                lines.len()
            )));
        }
        let before_start = start_idx.saturating_sub(window);
        let after_end = (end_idx + 1 + window).min(lines.len());
        Ok(Self {
            before: lines[before_start..start_idx]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            selected: lines[start_idx..=end_idx]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            after: lines[(end_idx + 1)..after_end]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        })
    }
}

/// Outcome of relocating an anchor's captured context against updated file
/// content. `new_start` is the 1-based line where the selection now begins;
/// `span` is the number of lines the (re)located selection covers, derived
/// from the matched content rather than trusted from the anchor's own prior
/// bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorRelocation {
    Current { new_start: u32, span: u32 },
    Stale,
    Ambiguous,
}

/// An exact anchor remap supplied by a provider adapter for an imported
/// remote thread — e.g. a GitHub review-thread's `line`/`originalLine`
/// fields already recomputed by the forge itself. When passed to
/// [`Anchor::relocate_with_remap`], this always wins over unique context
/// relocation, per the contract's "an exact provider remap wins first"
/// rule, even when context relocation alone would conclude `Stale` or
/// `Ambiguous`.
///
/// This type only carries the remap payload. It deliberately has no
/// knowledge of any real provider adapter, HTTP client, or persistence —
/// producing one is out of scope for this domain-only module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRemap {
    /// New 1-based start line (the line itself, for a `Line` anchor).
    pub new_start: u32,
    /// New inclusive end line for a `Range` anchor. Must be `None` when
    /// applied to a `Line` anchor and `Some` (and not before `new_start`)
    /// when applied to a `Range` anchor — [`Anchor::relocate_with_remap`]
    /// rejects any other combination rather than silently coercing it.
    pub new_end: Option<u32>,
}

impl ProviderRemap {
    pub fn line(new_line: u32) -> Self {
        Self {
            new_start: new_line,
            new_end: None,
        }
    }

    pub fn range(new_start: u32, new_end: u32) -> Self {
        Self {
            new_start,
            new_end: Some(new_end),
        }
    }
}

/// A normalized anchor: what it targets, its current resolution state, and
/// the context needed to relocate it after the file changes. `target` and
/// `state` are private — read them via [`Self::target`] and [`Self::state`]
/// — so they can only change through the relocation methods below, never by
/// direct field assignment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    target: AnchorTarget,
    state: AnchorState,
    pub context: Option<AnchorContext>,
}

impl Anchor {
    pub fn review() -> Self {
        Self {
            target: AnchorTarget::Review,
            state: AnchorState::Current,
            context: None,
        }
    }

    pub fn file(path: impl Into<String>) -> Self {
        Self {
            target: AnchorTarget::File { path: path.into() },
            state: AnchorState::Current,
            context: None,
        }
    }

    pub fn line(path: impl Into<String>, side: AnchorSide, line: u32) -> Self {
        Self {
            target: AnchorTarget::Line {
                path: path.into(),
                side,
                line,
            },
            state: AnchorState::Current,
            context: None,
        }
    }

    /// Construct a line anchor with captured relocation context. Returns
    /// [`TuicrError::InvalidInput`] if `context.selected` does not have
    /// exactly one line, since a line anchor's span is always 1.
    pub fn line_with_context(
        path: impl Into<String>,
        side: AnchorSide,
        line: u32,
        context: AnchorContext,
    ) -> Result<Self> {
        if context.selected.len() != 1 {
            return Err(TuicrError::InvalidInput(format!(
                "line anchor context must capture exactly 1 selected line, got {}",
                context.selected.len()
            )));
        }
        Ok(Self {
            target: AnchorTarget::Line {
                path: path.into(),
                side,
                line,
            },
            state: AnchorState::Current,
            context: Some(context),
        })
    }

    /// Construct a range anchor. Returns [`TuicrError::InvalidInput`] if
    /// `end` is before `start` — consistent with
    /// [`Self::range_with_context`], which rejects the same shape.
    pub fn range(path: impl Into<String>, side: AnchorSide, start: u32, end: u32) -> Result<Self> {
        if end < start {
            return Err(TuicrError::InvalidInput(format!(
                "range anchor end {end} is before start {start}"
            )));
        }
        Ok(Self {
            target: AnchorTarget::Range {
                path: path.into(),
                side,
                start,
                end,
            },
            state: AnchorState::Current,
            context: None,
        })
    }

    /// Construct a range anchor with captured relocation context. Returns
    /// [`TuicrError::InvalidInput`] if `end` is before `start`, or if
    /// `context.selected`'s length does not equal the range's span
    /// (`end - start + 1`).
    pub fn range_with_context(
        path: impl Into<String>,
        side: AnchorSide,
        start: u32,
        end: u32,
        context: AnchorContext,
    ) -> Result<Self> {
        if end < start {
            return Err(TuicrError::InvalidInput(format!(
                "range anchor end {end} is before start {start}"
            )));
        }
        let expected_span = (end - start + 1) as usize;
        if context.selected.len() != expected_span {
            return Err(TuicrError::InvalidInput(format!(
                "range anchor spans {expected_span} line(s) ({start}..={end}) but context \
                 captured {} selected line(s)",
                context.selected.len()
            )));
        }
        Ok(Self {
            target: AnchorTarget::Range {
                path: path.into(),
                side,
                start,
                end,
            },
            state: AnchorState::Current,
            context: Some(context),
        })
    }

    /// The anchor's current target (what it's anchored to). Read-only:
    /// mutate it only through [`Self::relocate`]/[`Self::relocate_with_remap`].
    pub fn target(&self) -> &AnchorTarget {
        &self.target
    }

    /// The anchor's current resolution state. Read-only: mutate it only
    /// through [`Self::relocate`]/[`Self::relocate_with_remap`].
    pub fn state(&self) -> AnchorState {
        self.state
    }

    pub fn is_current(&self) -> bool {
        self.state == AnchorState::Current
    }

    pub fn is_stale(&self) -> bool {
        self.state == AnchorState::Stale
    }

    pub fn is_ambiguous(&self) -> bool {
        self.state == AnchorState::Ambiguous
    }

    pub fn has_context(&self) -> bool {
        self.context.is_some()
    }

    /// Attach creation-time context to a context-free line/range anchor.
    pub fn attach_context(&mut self, context: AnchorContext) -> Result<()> {
        if self.context.is_some() {
            return Err(TuicrError::InvalidInput(
                "anchor already has relocation context".to_string(),
            ));
        }
        let expected_span = match &self.target {
            AnchorTarget::Line { .. } => 1,
            AnchorTarget::Range { start, end, .. } => (end - start + 1) as usize,
            AnchorTarget::Review | AnchorTarget::File { .. } => {
                return Err(TuicrError::InvalidInput(
                    "review and file anchors do not accept relocation context".to_string(),
                ));
            }
        };
        if context.selected.len() != expected_span {
            return Err(TuicrError::InvalidInput(format!(
                "anchor spans {expected_span} line(s) but context captured {} selected line(s)",
                context.selected.len()
            )));
        }
        self.context = Some(context);
        Ok(())
    }

    /// Monotonically apply a provider's explicit stale/outdated signal.
    ///
    /// Only a still-current anchor changes. Existing local `Stale` or
    /// `Ambiguous` results are preserved, and the target is never moved.
    pub fn mark_stale_from_provider(&mut self) -> bool {
        if self.state != AnchorState::Current {
            return false;
        }
        self.state = AnchorState::Stale;
        true
    }

    /// Relocate using unique context matching only. Equivalent to
    /// `relocate_with_remap(new_lines, None).expect(...)`; without a
    /// provider remap there is no remap/target shape to validate, so this
    /// never fails.
    pub fn relocate(&mut self, new_lines: &[&str]) -> AnchorRelocation {
        self.relocate_with_remap(new_lines, None)
            .expect("relocate without a provider remap never fails")
    }

    /// Relocate this anchor against `new_lines` (a 0-based full-file line
    /// slice), letting an exact `provider_remap` (when supplied) win over
    /// unique context relocation. On a unique match — or on any provider
    /// remap — moves the `Line`/`Range` target to its new position and sets
    /// `state` to `Current`; on zero context matches (and no remap), sets
    /// `state` to `Stale`; on more than one context match (and no remap),
    /// sets `state` to `Ambiguous`. Review/file anchors and anchors with no
    /// captured context or remap are left untouched (when no remap is
    /// supplied).
    ///
    /// A supplied `provider_remap` must match the anchor's target shape, or
    /// this returns `Err(TuicrError::InvalidInput)` without mutating the
    /// anchor at all — it never silently coerces a mismatched remap:
    /// - a `Range` target requires `provider_remap.new_end` to be `Some`
    ///   and not before `new_start`; a line-only remap (`new_end: None`)
    ///   cannot express a range and is rejected;
    /// - a `Line` target requires `provider_remap.new_end` to be `None`; a
    ///   range remap is rejected rather than silently discarding its
    ///   `new_end`;
    /// - `Review`/`File` targets have no line to move and reject any
    ///   `provider_remap`.
    pub fn relocate_with_remap(
        &mut self,
        new_lines: &[&str],
        provider_remap: Option<&ProviderRemap>,
    ) -> Result<AnchorRelocation> {
        if let Some(remap) = provider_remap {
            let span = match &self.target {
                AnchorTarget::Range { .. } => {
                    let new_end = remap.new_end.ok_or_else(|| {
                        TuicrError::InvalidInput(
                            "range anchor provider remap must supply new_end; a line-only \
                             remap (new_end: None) cannot relocate a range anchor"
                                .to_string(),
                        )
                    })?;
                    if new_end < remap.new_start {
                        return Err(TuicrError::InvalidInput(format!(
                            "range anchor provider remap new_end {new_end} is before \
                             new_start {}",
                            remap.new_start
                        )));
                    }
                    new_end - remap.new_start + 1
                }
                AnchorTarget::Line { .. } => {
                    if remap.new_end.is_some() {
                        return Err(TuicrError::InvalidInput(
                            "line anchor provider remap must not supply new_end; a range \
                             remap is not supported for a line anchor"
                                .to_string(),
                        ));
                    }
                    1
                }
                AnchorTarget::Review | AnchorTarget::File { .. } => {
                    return Err(TuicrError::InvalidInput(
                        "review/file anchors have no line to move and cannot accept a \
                         provider remap"
                            .to_string(),
                    ));
                }
            };
            self.state = AnchorState::Current;
            self.apply_current(remap.new_start, span);
            return Ok(AnchorRelocation::Current {
                new_start: remap.new_start,
                span,
            });
        }

        let Some(context) = self.context.as_ref() else {
            return Ok(match self.state {
                AnchorState::Current => {
                    let (new_start, span) = match &self.target {
                        AnchorTarget::Line { line, .. } => (*line, 1),
                        AnchorTarget::Range { start, end, .. } => {
                            (*start, end.saturating_sub(*start) + 1)
                        }
                        _ => (0, 1),
                    };
                    AnchorRelocation::Current { new_start, span }
                }
                AnchorState::Stale => AnchorRelocation::Stale,
                AnchorState::Ambiguous => AnchorRelocation::Ambiguous,
            });
        };

        let relocation = relocate_context(context, new_lines);
        match relocation {
            AnchorRelocation::Current { new_start, span } => {
                self.state = AnchorState::Current;
                self.apply_current(new_start, span);
            }
            AnchorRelocation::Stale => self.state = AnchorState::Stale,
            AnchorRelocation::Ambiguous => self.state = AnchorState::Ambiguous,
        }
        Ok(relocation)
    }

    /// Move a `Line`/`Range` target to `new_start` with the given `span`
    /// (derived from matched content, not from the anchor's prior state).
    /// No-op for `Review`/`File` targets.
    fn apply_current(&mut self, new_start: u32, span: u32) {
        match &mut self.target {
            AnchorTarget::Line { line, .. } => *line = new_start,
            AnchorTarget::Range { start, end, .. } => {
                *start = new_start;
                *end = new_start + span.saturating_sub(1);
            }
            _ => {}
        }
    }
}

/// Find every 0-based line index in `new_lines` where `context.selected`
/// appears with exactly the captured `before`/`after` context around it.
/// Returns `Stale` for zero matches, `Ambiguous` for more than one, and
/// `Current` only for a single unique match — this function never picks a
/// "closest" match when there is more than one. The returned span is always
/// `context.selected.len()`: the length of the content that actually
/// matched, not a value trusted from elsewhere.
fn relocate_context(context: &AnchorContext, new_lines: &[&str]) -> AnchorRelocation {
    let selected_len = context.selected.len().max(1);
    let before_len = context.before.len();
    let after_len = context.after.len();

    let mut matches = Vec::new();
    if selected_len == 0 || new_lines.len() < selected_len {
        return AnchorRelocation::Stale;
    }

    for start in 0..=(new_lines.len() - selected_len) {
        if before_len > start {
            continue;
        }
        let before_start = start - before_len;
        if new_lines[before_start..start] != context.before[..] {
            continue;
        }
        if new_lines[start..start + selected_len] != context.selected[..] {
            continue;
        }
        let after_end = start + selected_len + after_len;
        if after_end > new_lines.len() {
            continue;
        }
        if new_lines[(start + selected_len)..after_end] != context.after[..] {
            continue;
        }
        matches.push(start);
    }

    match matches.as_slice() {
        [] => AnchorRelocation::Stale,
        [only] => AnchorRelocation::Current {
            new_start: (*only as u32) + 1,
            span: selected_len as u32,
        },
        _ => AnchorRelocation::Ambiguous,
    }
}

/// Lifecycle state of a [`Thread`]. Mirrors
/// `schemas/review-artifact-v1.schema.json#/$defs/thread.status`:
/// `open <-> resolved`, with `stale`/`ambiguous`/`dismissed` as one-way
/// branches off `open` driven by anchor resolution or explicit dismissal.
/// Only `Resolved` has a return arrow (`reopen`); `Dismissed` is terminal —
/// once dismissed, a thread can never be resolved or reopened again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    Open,
    Resolved,
    Stale,
    Ambiguous,
    /// Terminal: a dismissed thread's `reopen`/`resolve` are permanent
    /// no-ops, and its anchor is frozen forever (see
    /// [`Thread::refresh_anchor_with_remap`]).
    Dismissed,
}

/// Outcome of asking a [`Thread`] to refresh its anchor. Distinguishes an
/// anchor that was actually re-evaluated from one left fully untouched
/// because the thread is closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadAnchorRefresh {
    /// The anchor was re-evaluated; wraps the resulting [`AnchorRelocation`].
    Applied(AnchorRelocation),
    /// The thread is closed (`Resolved` or `Dismissed`), so the anchor's
    /// `state` and `target` were left byte-for-byte unchanged: closed
    /// threads never call into relocation at all. An explicit
    /// provider-outdated signal can still mark a current anchor stale without
    /// moving its target via [`Thread::mark_stale_from_provider`]. Reopening a
    /// `Resolved` thread resumes anchor churn; `Dismissed` is terminal and can
    /// never be reopened, so its target freeze is permanent.
    Frozen,
}

/// A discussion anchored at one place in a review: a stable ID, its current
/// anchor, and its ordered comments (root first, then replies). `id`,
/// `status`, `anchor`, and `comments` are private — read them via
/// [`Self::id`], [`Self::status`], [`Self::anchor`], and [`Self::comments`]
/// (or the narrower [`Self::root`]/[`Self::replies`]) — so lifecycle
/// transitions, anchor relocation, and comment append can only happen
/// through the tested methods below, never by reaching in and assigning a
/// field, calling a relocation method directly (bypassing the
/// closed-thread freeze), or mutating the comment vector directly
/// (bypassing [`Self::reply`], e.g. to reorder or remove the root).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    id: ThreadId,
    status: ThreadStatus,
    anchor: Anchor,
    comments: Vec<ThreadComment>,
}

impl Thread {
    /// Open a new thread with `root` as its first comment.
    pub fn open(anchor: Anchor, root: ThreadComment) -> Self {
        Self {
            id: ThreadId::new(),
            status: ThreadStatus::Open,
            anchor,
            comments: vec![root],
        }
    }

    pub fn id(&self) -> &ThreadId {
        &self.id
    }

    pub fn status(&self) -> ThreadStatus {
        self.status
    }

    pub fn anchor(&self) -> &Anchor {
        &self.anchor
    }

    pub fn attach_anchor_context(&mut self, context: AnchorContext) -> Result<()> {
        self.anchor.attach_context(context)
    }

    /// All comments in the thread, root first, then replies in append
    /// order. Read-only: the only way to add to this list is
    /// [`Self::reply`].
    pub fn comments(&self) -> &[ThreadComment] {
        &self.comments
    }

    pub fn root(&self) -> Option<&ThreadComment> {
        self.comments.first()
    }

    pub fn replies(&self) -> impl Iterator<Item = &ThreadComment> {
        self.comments.iter().skip(1)
    }

    /// Keep the root and only replies accepted by `keep`.
    pub(crate) fn retain_replies(&mut self, mut keep: impl FnMut(&ThreadComment) -> bool) {
        let mut is_root = true;
        self.comments.retain(|comment| {
            if is_root {
                is_root = false;
                true
            } else {
                keep(comment)
            }
        });
    }

    /// Append a reply, returning its immutable ID.
    pub fn reply(&mut self, comment: ThreadComment) -> CommentId {
        let id = comment.id().clone();
        self.comments.push(comment);
        id
    }

    /// Mark the thread resolved. Valid from any non-dismissed status.
    /// Returns `false` (no-op) if already dismissed.
    pub fn resolve(&mut self) -> bool {
        if self.status == ThreadStatus::Dismissed {
            return false;
        }
        self.status = ThreadStatus::Resolved;
        true
    }

    /// Reopen a resolved thread. Only valid from `Resolved`; `Dismissed` is
    /// terminal (it means "won't fix") and never reopens. Returns `false`
    /// (no-op) if the thread was not `Resolved`, including when it is
    /// `Dismissed`.
    ///
    /// The anchor was frozen while the thread was closed, so its `state`
    /// may no longer be `Current` (e.g. it went `Stale`/`Ambiguous` before
    /// being resolved, and nothing re-evaluated it while closed). Reopening
    /// re-derives the thread's status from that frozen `anchor.state`
    /// rather than unconditionally forcing `Open`, so a thread that was
    /// already `Stale`/`Ambiguous` when resolved comes back `Stale`/
    /// `Ambiguous`, not a false `Open`.
    pub fn reopen(&mut self) -> bool {
        if self.status != ThreadStatus::Resolved {
            return false;
        }
        self.status = match self.anchor.state() {
            AnchorState::Current => ThreadStatus::Open,
            AnchorState::Stale => ThreadStatus::Stale,
            AnchorState::Ambiguous => ThreadStatus::Ambiguous,
        };
        true
    }

    /// Mark the thread dismissed ("won't fix"). Terminal: a dismissed
    /// thread can never be resolved or reopened again (see [`Self::resolve`]
    /// and [`Self::reopen`]), and its anchor is permanently frozen (see
    /// [`Self::refresh_anchor_with_remap`]).
    pub fn dismiss(&mut self) {
        self.status = ThreadStatus::Dismissed;
    }

    pub fn is_open(&self) -> bool {
        self.status == ThreadStatus::Open
    }

    pub fn is_resolved(&self) -> bool {
        self.status == ThreadStatus::Resolved
    }

    /// Monotonically apply an explicit provider-outdated signal without
    /// guessing a new target.
    ///
    /// Open-family threads become `Stale`. Resolved/dismissed threads retain
    /// their closed status but keep the stale anchor state, so reopening a
    /// resolved thread correctly returns it to `Stale`. Existing local stale
    /// or ambiguous anchor results always win.
    pub fn mark_stale_from_provider(&mut self) -> bool {
        if !self.anchor.mark_stale_from_provider() {
            return false;
        }
        if !matches!(
            self.status,
            ThreadStatus::Resolved | ThreadStatus::Dismissed
        ) {
            self.status = ThreadStatus::Stale;
        }
        true
    }

    /// Re-run anchor relocation against updated file content using unique
    /// context matching only. Equivalent to
    /// `refresh_anchor_with_remap(new_lines, None).expect(...)`; without a
    /// provider remap there is no remap/target shape to validate, so this
    /// never fails.
    pub fn refresh_anchor(&mut self, new_lines: &[&str]) -> ThreadAnchorRefresh {
        self.refresh_anchor_with_remap(new_lines, None)
            .expect("refresh_anchor without a provider remap never fails")
    }

    /// Re-run anchor relocation against updated file content, letting an
    /// exact `provider_remap` (when supplied) win over unique context
    /// relocation.
    ///
    /// Closed threads (`Resolved`/`Dismissed`) are frozen against relocation:
    /// this returns `Ok(Frozen)` immediately, without calling into
    /// `Anchor::relocate_with_remap` at all, so neither the anchor's `state`
    /// nor its `target` are touched — even if `provider_remap` is supplied.
    /// The separate [`Self::mark_stale_from_provider`] path may mark state
    /// stale but never changes the target. Reopening a `Resolved` thread
    /// resumes anchor churn; `Dismissed` is terminal, so its target freeze is
    /// permanent.
    ///
    /// For open-family threads (`Open`, `Stale`, `Ambiguous`), the anchor is
    /// re-evaluated and the thread's status follows the resulting anchor
    /// state. Returns `Err(TuicrError::InvalidInput)` if `provider_remap` is
    /// supplied but its shape does not match the anchor's target (see
    /// [`Anchor::relocate_with_remap`]).
    pub fn refresh_anchor_with_remap(
        &mut self,
        new_lines: &[&str],
        provider_remap: Option<&ProviderRemap>,
    ) -> Result<ThreadAnchorRefresh> {
        if matches!(
            self.status,
            ThreadStatus::Resolved | ThreadStatus::Dismissed
        ) {
            return Ok(ThreadAnchorRefresh::Frozen);
        }
        let relocation = self.anchor.relocate_with_remap(new_lines, provider_remap)?;
        self.status = match relocation {
            AnchorRelocation::Current { .. } => ThreadStatus::Open,
            AnchorRelocation::Stale => ThreadStatus::Stale,
            AnchorRelocation::Ambiguous => ThreadStatus::Ambiguous,
        };
        Ok(ThreadAnchorRefresh::Applied(relocation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_no(n: usize) -> String {
        format!("line_{n:04}")
    }

    /// A synthetic file where every line's content is unique, so any single
    /// line is trivially a unique context match by itself.
    fn unique_file(len: usize) -> Vec<String> {
        (1..=len).map(line_no).collect()
    }

    fn as_str_slice(lines: &[String]) -> Vec<&str> {
        lines.iter().map(|s| s.as_str()).collect()
    }

    mod identity_tests {
        use super::*;

        #[test]
        fn thread_ids_are_unique_and_stable_across_mutation() {
            let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
            let root = ThreadComment::new(ThreadAuthor::human("alice"), "root");
            let mut thread = Thread::open(anchor, root);
            let id_before = thread.id().clone();

            thread.reply(ThreadComment::new(ThreadAuthor::human("bob"), "reply"));
            thread.resolve();
            thread.reopen();

            assert_eq!(
                thread.id(),
                &id_before,
                "thread id must not change across mutation"
            );
        }

        #[test]
        fn two_threads_get_different_ids() {
            let a = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "a"),
            );
            let b = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "b"),
            );
            assert_ne!(a.id(), b.id());
        }

        #[test]
        fn comment_ids_are_immutable_and_unique() {
            let root = ThreadComment::new(ThreadAuthor::human("alice"), "root");
            let reply = ThreadComment::new(ThreadAuthor::agent("copilot"), "reply");
            assert_ne!(root.id(), reply.id());

            let mut thread = Thread::open(Anchor::review(), root.clone());
            let returned_id = thread.reply(reply.clone());
            assert_eq!(&returned_id, reply.id());
            assert_eq!(thread.comments()[0].id(), root.id());
            assert_eq!(thread.comments()[1].id(), reply.id());
        }

        #[test]
        fn thread_and_comment_ids_survive_json_roundtrip() {
            let mut thread = Thread::open(
                Anchor::line("src/a.rs", AnchorSide::New, 5),
                ThreadComment::new(ThreadAuthor::human("alice"), "root"),
            );
            thread.reply(ThreadComment::new(ThreadAuthor::agent("copilot"), "reply"));

            let json = serde_json::to_string(&thread).unwrap();
            let restored: Thread = serde_json::from_str(&json).unwrap();
            assert_eq!(restored.id(), thread.id());
            assert_eq!(restored.comments()[0].id(), thread.comments()[0].id());
            assert_eq!(restored.comments()[1].id(), thread.comments()[1].id());
        }
    }

    mod authorship_tests {
        use super::*;

        #[test]
        fn human_author_is_human_not_agent() {
            let author = ThreadAuthor::human("alice");
            assert!(author.is_human());
            assert!(!author.is_agent());
            assert_eq!(author.kind, AuthorKind::Human);
        }

        #[test]
        fn agent_author_is_agent_not_human() {
            let author = ThreadAuthor::agent("copilot");
            assert!(author.is_agent());
            assert!(!author.is_human());
            assert_eq!(author.kind, AuthorKind::Agent);
        }

        #[test]
        fn remote_author_carries_provider_id() {
            let author = ThreadAuthor::remote("octocat", "MDQ6VXNlcjE=");
            assert_eq!(author.kind, AuthorKind::Remote);
            assert_eq!(author.provider_id.as_deref(), Some("MDQ6VXNlcjE="));
        }

        #[test]
        fn author_kind_serializes_to_schema_compatible_strings() {
            assert_eq!(
                serde_json::to_string(&AuthorKind::Human).unwrap(),
                "\"human\""
            );
            assert_eq!(
                serde_json::to_string(&AuthorKind::Agent).unwrap(),
                "\"agent\""
            );
            assert_eq!(
                serde_json::to_string(&AuthorKind::Remote).unwrap(),
                "\"remote\""
            );
        }

        #[test]
        fn a_thread_mixes_human_and_agent_replies() {
            let mut thread = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "please explain this"),
            );
            thread.reply(ThreadComment::new(
                ThreadAuthor::agent("copilot"),
                "here is the rationale",
            ));
            assert!(thread.root().unwrap().author.is_human());
            assert!(thread.replies().next().unwrap().author.is_agent());
        }
    }

    mod lifecycle_tests {
        use super::*;

        #[test]
        fn new_thread_starts_open() {
            let thread = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "hi"),
            );
            assert_eq!(thread.status(), ThreadStatus::Open);
            assert!(thread.is_open());
            assert!(!thread.is_resolved());
        }

        #[test]
        fn resolve_transitions_open_to_resolved() {
            let mut thread = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "hi"),
            );
            assert!(thread.resolve());
            assert_eq!(thread.status(), ThreadStatus::Resolved);
            assert!(thread.is_resolved());
        }

        #[test]
        fn reopen_transitions_resolved_back_to_open() {
            let mut thread = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "hi"),
            );
            thread.resolve();
            assert!(thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Open);
        }

        #[test]
        fn reopen_is_a_noop_when_not_resolved() {
            let mut thread = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "hi"),
            );
            assert!(!thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Open);
        }

        #[test]
        fn dismiss_locks_out_further_resolve() {
            let mut thread = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "hi"),
            );
            thread.dismiss();
            assert_eq!(thread.status(), ThreadStatus::Dismissed);
            assert!(!thread.resolve());
            assert_eq!(thread.status(), ThreadStatus::Dismissed);
        }

        #[test]
        fn dismissed_is_terminal_and_can_never_be_reopened() {
            let mut thread = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "hi"),
            );
            thread.dismiss();

            // Unlike Resolved, Dismissed never accepts reopen — not once,
            // and not on repeated attempts.
            assert!(!thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Dismissed);
            assert!(!thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Dismissed);
        }

        #[test]
        fn reopen_from_resolved_succeeds_but_dismissed_does_not() {
            let mut resolved = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "hi"),
            );
            resolved.resolve();
            assert!(resolved.reopen(), "reopen must work from Resolved");
            assert_eq!(resolved.status(), ThreadStatus::Open);

            let mut dismissed = Thread::open(
                Anchor::review(),
                ThreadComment::new(ThreadAuthor::human("alice"), "hi"),
            );
            dismissed.dismiss();
            assert!(
                !dismissed.reopen(),
                "reopen must be a permanent no-op from Dismissed"
            );
            assert_eq!(dismissed.status(), ThreadStatus::Dismissed);
        }

        #[test]
        fn provider_stale_is_monotonic_and_resolved_reopens_stale() {
            let anchor = Anchor::line("src/lib.rs", AnchorSide::New, 42);
            let mut thread = Thread::open(
                anchor,
                ThreadComment::new(ThreadAuthor::remote("alice", "alice"), "review"),
            );

            assert!(thread.mark_stale_from_provider());
            assert_eq!(thread.status(), ThreadStatus::Stale);
            assert!(thread.anchor().is_stale());
            assert!(!thread.mark_stale_from_provider());
            assert_eq!(
                thread.refresh_anchor(&["unrelated content"]),
                ThreadAnchorRefresh::Applied(AnchorRelocation::Stale)
            );
            assert_eq!(thread.status(), ThreadStatus::Stale);
            assert!(thread.anchor().is_stale());

            thread.resolve();
            assert_eq!(thread.status(), ThreadStatus::Resolved);
            assert!(thread.anchor().is_stale());
            assert!(thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Stale);
        }

        #[test]
        fn provider_stale_does_not_replace_an_ambiguous_anchor() {
            let anchor: Anchor = serde_json::from_value(serde_json::json!({
                "target": {
                    "kind": "line",
                    "path": "src/lib.rs",
                    "side": "new",
                    "line": 42
                },
                "state": "ambiguous",
                "context": null
            }))
            .unwrap();
            let mut thread = Thread {
                id: ThreadId::new(),
                status: ThreadStatus::Ambiguous,
                anchor,
                comments: vec![ThreadComment::new(
                    ThreadAuthor::remote("alice", "alice"),
                    "review",
                )],
            };

            assert!(!thread.mark_stale_from_provider());
            assert!(thread.anchor().is_ambiguous());
            assert_eq!(thread.status(), ThreadStatus::Ambiguous);
        }
    }

    mod anchor_kind_tests {
        use super::*;

        #[test]
        fn review_anchor_has_no_path_or_context() {
            let anchor = Anchor::review();
            assert_eq!(anchor.target(), &AnchorTarget::Review);
            assert_eq!(anchor.target().path(), None);
            assert!(anchor.is_current());
        }

        #[test]
        fn file_anchor_carries_a_path() {
            let anchor = Anchor::file("src/lib.rs");
            assert_eq!(anchor.target().path(), Some("src/lib.rs"));
            assert!(matches!(anchor.target(), AnchorTarget::File { .. }));
        }

        #[test]
        fn old_side_line_anchor() {
            let anchor = Anchor::line("src/legacy/obsolete.ts", AnchorSide::Old, 20);
            match anchor.target().clone() {
                AnchorTarget::Line { side, line, .. } => {
                    assert_eq!(side, AnchorSide::Old);
                    assert_eq!(line, 20);
                }
                _ => panic!("expected a line anchor"),
            }
        }

        #[test]
        fn new_side_line_anchor() {
            let anchor = Anchor::line("src/review-policy.ts", AnchorSide::New, 42);
            match anchor.target().clone() {
                AnchorTarget::Line { side, line, .. } => {
                    assert_eq!(side, AnchorSide::New);
                    assert_eq!(line, 42);
                }
                _ => panic!("expected a line anchor"),
            }
        }

        #[test]
        fn range_anchor_carries_inclusive_bounds_and_side() {
            let anchor =
                Anchor::range("src/catalog/service_03.ts", AnchorSide::New, 30, 35).unwrap();
            match anchor.target().clone() {
                AnchorTarget::Range {
                    side, start, end, ..
                } => {
                    assert_eq!(side, AnchorSide::New);
                    assert_eq!(start, 30);
                    assert_eq!(end, 35);
                }
                _ => panic!("expected a range anchor"),
            }
        }

        #[test]
        fn anchor_state_defaults_to_current_for_freshly_created_anchors() {
            for anchor in [
                Anchor::review(),
                Anchor::file("f.ts"),
                Anchor::line("f.ts", AnchorSide::New, 1),
                Anchor::range("f.ts", AnchorSide::New, 1, 2).unwrap(),
            ] {
                assert_eq!(anchor.state(), AnchorState::Current);
            }
        }
    }

    mod construction_validation_tests {
        use super::*;

        #[test]
        fn capture_rejects_start_idx_after_end_idx() {
            let file = unique_file(20);
            let refs = as_str_slice(&file);
            let err = AnchorContext::capture(&refs, 10, 5, 2).unwrap_err();
            assert!(matches!(err, TuicrError::InvalidInput(_)));
        }

        #[test]
        fn capture_rejects_out_of_bounds_end_idx() {
            let file = unique_file(20);
            let refs = as_str_slice(&file);
            let err = AnchorContext::capture(&refs, 5, 1000, 2).unwrap_err();
            assert!(matches!(err, TuicrError::InvalidInput(_)));
        }

        #[test]
        fn capture_succeeds_for_valid_single_line_selection() {
            let file = unique_file(20);
            let refs = as_str_slice(&file);
            let context = AnchorContext::capture(&refs, 9, 9, 2).unwrap();
            assert_eq!(context.selected, vec![line_no(10)]);
        }

        #[test]
        fn line_with_context_rejects_multi_line_selected_context() {
            let file = unique_file(20);
            let refs = as_str_slice(&file);
            // A 2-line selected block for a single-line anchor is a
            // span/context mismatch.
            let context = AnchorContext::capture(&refs, 9, 10, 2).unwrap();
            let err = Anchor::line_with_context("f.ts", AnchorSide::New, 10, context).unwrap_err();
            assert!(matches!(err, TuicrError::InvalidInput(_)));
        }

        #[test]
        fn range_with_context_rejects_end_before_start() {
            let file = unique_file(20);
            let refs = as_str_slice(&file);
            let context = AnchorContext::capture(&refs, 9, 9, 2).unwrap();
            let err =
                Anchor::range_with_context("f.ts", AnchorSide::New, 10, 5, context).unwrap_err();
            assert!(matches!(err, TuicrError::InvalidInput(_)));
        }

        #[test]
        fn range_rejects_end_before_start_consistently_with_range_with_context() {
            // Anchor::range (no captured context) must reject the same
            // end < start shape as Anchor::range_with_context, rather than
            // silently coercing it (e.g. via saturating arithmetic).
            let err = Anchor::range("f.ts", AnchorSide::New, 10, 5).unwrap_err();
            assert!(matches!(err, TuicrError::InvalidInput(_)));
        }

        #[test]
        fn range_accepts_a_single_line_span_where_end_equals_start() {
            // end == start is a valid (degenerate) 1-line range, not
            // rejected by the end < start check.
            let anchor = Anchor::range("f.ts", AnchorSide::New, 10, 10).unwrap();
            match anchor.target().clone() {
                AnchorTarget::Range { start, end, .. } => {
                    assert_eq!(start, 10);
                    assert_eq!(end, 10);
                }
                _ => panic!("expected a range anchor"),
            }
        }

        #[test]
        fn range_with_context_rejects_selected_length_diverging_from_span() {
            let file = unique_file(20);
            let refs = as_str_slice(&file);
            // Anchor claims a 6-line span (30..=35 in 1-based == 29..=34 in
            // 0-based), but only 5 lines of context are actually captured.
            let context = AnchorContext::capture(&refs, 9, 13, 2).unwrap(); // 5 lines
            assert_eq!(context.selected.len(), 5);
            let err =
                Anchor::range_with_context("f.ts", AnchorSide::New, 30, 35, context).unwrap_err();
            assert!(matches!(err, TuicrError::InvalidInput(_)));
        }

        #[test]
        fn range_with_context_accepts_matching_span() {
            let file = unique_file(20);
            let refs = as_str_slice(&file);
            let context = AnchorContext::capture(&refs, 9, 14, 2).unwrap(); // 6 lines
            assert_eq!(context.selected.len(), 6);
            let anchor =
                Anchor::range_with_context("f.ts", AnchorSide::New, 30, 35, context).unwrap();
            assert_eq!(anchor.state(), AnchorState::Current);
        }
    }

    mod relocation_tests {
        use super::*;

        /// Mirrors the common fixture's `new-line-1` comment: a unique-context
        /// anchor at line 42 of `src/review-policy.ts` relocates to line 47
        /// after a five-line insertion at the top of the file.
        #[test]
        fn unique_context_relocates_line_42_to_47_after_five_line_insertion() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap(); // 0-based line 42
            let mut anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();

            let mut updated: Vec<String> = (1..=5).map(|n| format!("inserted_{n}")).collect();
            updated.extend(original.clone());
            let updated_refs = as_str_slice(&updated);

            let relocation = anchor.relocate(&updated_refs);
            assert_eq!(
                relocation,
                AnchorRelocation::Current {
                    new_start: 47,
                    span: 1
                }
            );
            assert_eq!(anchor.state(), AnchorState::Current);
            match anchor.target().clone() {
                AnchorTarget::Line { line, .. } => assert_eq!(line, 47),
                _ => panic!("expected a line anchor"),
            }
        }

        #[test]
        fn relocation_also_shifts_a_range_anchor_preserving_its_span() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            // 0-based lines 29..=34 == 1-based 30..=35, a 6-line range.
            let context = AnchorContext::capture(&original_refs, 29, 34, 2).unwrap();
            let mut anchor = Anchor::range_with_context(
                "src/catalog/service_03.ts",
                AnchorSide::New,
                30,
                35,
                context,
            )
            .unwrap();

            let mut updated: Vec<String> = (1..=5).map(|n| format!("inserted_{n}")).collect();
            updated.extend(original.clone());
            let updated_refs = as_str_slice(&updated);

            let relocation = anchor.relocate(&updated_refs);
            assert_eq!(
                relocation,
                AnchorRelocation::Current {
                    new_start: 35,
                    span: 6
                }
            );
            match anchor.target().clone() {
                AnchorTarget::Range { start, end, .. } => {
                    assert_eq!(start, 35);
                    assert_eq!(end, 40);
                }
                _ => panic!("expected a range anchor"),
            }
        }

        #[test]
        fn zero_matches_after_context_is_removed_marks_anchor_stale() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            let mut anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();

            // The whole neighborhood around the old anchor line is gone.
            let updated: Vec<String> = original
                .iter()
                .enumerate()
                .filter(|(i, _)| !(38..=44).contains(i))
                .map(|(_, l)| l.clone())
                .collect();
            let updated_refs = as_str_slice(&updated);

            let relocation = anchor.relocate(&updated_refs);
            assert_eq!(relocation, AnchorRelocation::Stale);
            assert_eq!(anchor.state(), AnchorState::Stale);
            // The stale target keeps its last-known line rather than guessing.
            match anchor.target().clone() {
                AnchorTarget::Line { line, .. } => assert_eq!(line, 42),
                _ => panic!("expected a line anchor"),
            }
        }

        #[test]
        fn multiple_matches_marks_anchor_ambiguous_never_nearest_line() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            let mut anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();

            // Duplicate the exact same 5-line context window (lines 40..=44,
            // 1-based) far away in the file, at both a nearer and a farther
            // offset, so a "nearest line" heuristic would be tempted to pick
            // the closer duplicate. The contract forbids that: any count > 1
            // must be `Ambiguous`, regardless of proximity.
            let duplicate: Vec<String> = original[39..44].to_vec();
            let mut updated = original.clone();
            // Insert the near duplicate right after the original block, and a
            // far duplicate near the end of the file.
            updated.splice(50..50, duplicate.clone());
            updated.splice(90..90, duplicate.clone());
            let updated_refs = as_str_slice(&updated);

            let relocation = anchor.relocate(&updated_refs);
            assert_eq!(relocation, AnchorRelocation::Ambiguous);
            assert_eq!(anchor.state(), AnchorState::Ambiguous);
            match anchor.target().clone() {
                AnchorTarget::Line { line, .. } => assert_eq!(line, 42),
                _ => panic!("expected a line anchor"),
            }
        }

        #[test]
        fn anchor_without_context_relocates_to_its_own_line_unchanged() {
            let mut anchor = Anchor::line("src/a.rs", AnchorSide::New, 7);
            let file = unique_file(20);
            let lines = as_str_slice(&file);
            let relocation = anchor.relocate(&lines);
            assert_eq!(
                relocation,
                AnchorRelocation::Current {
                    new_start: 7,
                    span: 1
                }
            );
        }

        #[test]
        fn thread_refresh_anchor_moves_open_thread_between_lifecycle_and_anchor_states() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            let anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();
            let mut thread = Thread::open(
                anchor,
                ThreadComment::new(ThreadAuthor::human("alice"), "explain rule 42"),
            );

            // Context vanishes -> thread becomes Stale, still Open-family.
            let stale_update: Vec<String> = original
                .iter()
                .enumerate()
                .filter(|(i, _)| !(38..=44).contains(i))
                .map(|(_, l)| l.clone())
                .collect();
            let refresh = thread.refresh_anchor(&as_str_slice(&stale_update));
            assert_eq!(
                refresh,
                ThreadAnchorRefresh::Applied(AnchorRelocation::Stale)
            );
            assert_eq!(thread.status(), ThreadStatus::Stale);
            assert!(thread.anchor().is_stale());

            // Context reappears uniquely -> thread recovers to Open/Current.
            let mut recovered: Vec<String> = (1..=5).map(|n| format!("inserted_{n}")).collect();
            recovered.extend(original.clone());
            let refresh = thread.refresh_anchor(&as_str_slice(&recovered));
            assert_eq!(
                refresh,
                ThreadAnchorRefresh::Applied(AnchorRelocation::Current {
                    new_start: 47,
                    span: 1
                })
            );
            assert_eq!(thread.status(), ThreadStatus::Open);
            assert!(thread.anchor().is_current());
        }

        #[test]
        fn reopen_re_derives_stale_status_from_frozen_anchor_state() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            let anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();
            let mut thread = Thread::open(
                anchor,
                ThreadComment::new(ThreadAuthor::human("alice"), "explain rule 42"),
            );

            // Drive the thread to Stale before resolving it.
            let stale_update: Vec<String> = original
                .iter()
                .enumerate()
                .filter(|(i, _)| !(38..=44).contains(i))
                .map(|(_, l)| l.clone())
                .collect();
            thread.refresh_anchor(&as_str_slice(&stale_update));
            assert_eq!(thread.status(), ThreadStatus::Stale);
            assert!(thread.anchor().is_stale());

            // A thread can be resolved even while its anchor is stale (e.g.
            // "not worth chasing"); resolving must not touch the frozen
            // anchor.
            assert!(thread.resolve());
            assert_eq!(thread.status(), ThreadStatus::Resolved);
            assert!(thread.anchor().is_stale());

            // Reopening must restore Stale, not fabricate a false Open: the
            // anchor never became Current while the thread was closed.
            assert!(thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Stale);
            assert!(thread.anchor().is_stale());
        }

        #[test]
        fn reopen_re_derives_ambiguous_status_from_frozen_anchor_state() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            let anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();
            let mut thread = Thread::open(
                anchor,
                ThreadComment::new(ThreadAuthor::human("alice"), "explain rule 42"),
            );

            // Drive the thread to Ambiguous before resolving it.
            let duplicate: Vec<String> = original[39..44].to_vec();
            let mut ambiguous_update = original.clone();
            ambiguous_update.splice(50..50, duplicate.clone());
            ambiguous_update.splice(90..90, duplicate);
            thread.refresh_anchor(&as_str_slice(&ambiguous_update));
            assert_eq!(thread.status(), ThreadStatus::Ambiguous);
            assert!(thread.anchor().is_ambiguous());

            assert!(thread.resolve());
            assert_eq!(thread.status(), ThreadStatus::Resolved);
            assert!(thread.anchor().is_ambiguous());

            // Reopening must restore Ambiguous, not fabricate a false Open.
            assert!(thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Ambiguous);
            assert!(thread.anchor().is_ambiguous());
        }
    }

    mod freeze_tests {
        use super::*;

        fn ambiguous_lines(original: &[String], recovered: &[String]) -> Vec<String> {
            let duplicate: Vec<String> = original[39..44].to_vec();
            let mut updated = recovered.to_vec();
            updated.splice(60..60, duplicate.clone());
            updated.splice(90..90, duplicate);
            updated
        }

        fn line_42_anchor_over(original: &[String]) -> Anchor {
            let original_refs = as_str_slice(original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context).unwrap()
        }

        #[test]
        fn resolved_thread_freezes_anchor_state_and_target_until_reopened() {
            let original = unique_file(100);
            let anchor = line_42_anchor_over(&original);
            let mut thread = Thread::open(
                anchor,
                ThreadComment::new(ThreadAuthor::human("alice"), "explain rule 42"),
            );

            let mut recovered: Vec<String> = (1..=5).map(|n| format!("inserted_{n}")).collect();
            recovered.extend(original.clone());
            thread.refresh_anchor(&as_str_slice(&recovered));
            assert_eq!(thread.status(), ThreadStatus::Open);
            assert!(thread.anchor().is_current());

            thread.resolve();
            let anchor_before = thread.anchor().clone();

            // Anchor churn that would otherwise mark the anchor Ambiguous
            // (and, separately, content that would relocate it) must not
            // touch a Resolved thread's anchor at all.
            let churned = ambiguous_lines(&original, &recovered);
            let refresh = thread.refresh_anchor(&as_str_slice(&churned));

            assert_eq!(refresh, ThreadAnchorRefresh::Frozen);
            assert_eq!(thread.status(), ThreadStatus::Resolved);
            assert_eq!(
                thread.anchor(),
                &anchor_before,
                "resolved thread must freeze both anchor state and target"
            );
            assert_eq!(thread.anchor().state(), AnchorState::Current);
            match thread.anchor().target().clone() {
                AnchorTarget::Line { line, .. } => assert_eq!(line, 47),
                _ => panic!("expected a line anchor"),
            }

            // Reopening resumes anchor churn: the same input now applies.
            assert!(thread.reopen());
            let refresh = thread.refresh_anchor(&as_str_slice(&churned));
            assert_eq!(
                refresh,
                ThreadAnchorRefresh::Applied(AnchorRelocation::Ambiguous)
            );
            assert_eq!(thread.status(), ThreadStatus::Ambiguous);
            assert!(thread.anchor().is_ambiguous());
        }

        #[test]
        fn dismissed_thread_freezes_anchor_state_and_target() {
            let original = unique_file(100);
            let anchor = line_42_anchor_over(&original);
            let mut thread = Thread::open(
                anchor,
                ThreadComment::new(ThreadAuthor::human("alice"), "explain rule 42"),
            );
            thread.dismiss();
            let anchor_before = thread.anchor().clone();

            // Even content that would relocate the anchor must not touch a
            // dismissed thread.
            let mut recovered: Vec<String> = (1..=5).map(|n| format!("inserted_{n}")).collect();
            recovered.extend(original.clone());
            let refresh = thread.refresh_anchor(&as_str_slice(&recovered));

            assert_eq!(refresh, ThreadAnchorRefresh::Frozen);
            assert_eq!(thread.status(), ThreadStatus::Dismissed);
            assert_eq!(
                thread.anchor(),
                &anchor_before,
                "dismissed thread must freeze both anchor state and target"
            );
            match thread.anchor().target().clone() {
                AnchorTarget::Line { line, .. } => assert_eq!(line, 42),
                _ => panic!("expected a line anchor"),
            }

            // Dismissed is terminal: reopen is a permanent no-op, and the
            // freeze is unaffected by further attempted churn.
            assert!(!thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Dismissed);
            let refresh_again = thread.refresh_anchor(&as_str_slice(&recovered));
            assert_eq!(refresh_again, ThreadAnchorRefresh::Frozen);
            assert_eq!(
                thread.anchor(),
                &anchor_before,
                "dismissed thread's freeze is permanent, not just until the next reopen attempt"
            );
        }
    }

    mod provider_remap_tests {
        use super::*;

        #[test]
        fn provider_remap_wins_over_context_that_would_be_ambiguous() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            let mut anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();

            // Build content where context relocation alone would be
            // Ambiguous (duplicated context at two offsets).
            let duplicate: Vec<String> = original[39..44].to_vec();
            let mut updated = original.clone();
            updated.splice(50..50, duplicate.clone());
            updated.splice(90..90, duplicate);
            let updated_refs = as_str_slice(&updated);

            // Sanity check: context relocation alone really is ambiguous.
            assert_eq!(
                relocate_context(anchor.context.as_ref().unwrap(), &updated_refs),
                AnchorRelocation::Ambiguous
            );

            let remap = ProviderRemap::line(200);
            let relocation = anchor
                .relocate_with_remap(&updated_refs, Some(&remap))
                .unwrap();
            assert_eq!(
                relocation,
                AnchorRelocation::Current {
                    new_start: 200,
                    span: 1
                }
            );
            assert_eq!(anchor.state(), AnchorState::Current);
            match anchor.target().clone() {
                AnchorTarget::Line { line, .. } => assert_eq!(line, 200),
                _ => panic!("expected a line anchor"),
            }
        }

        #[test]
        fn provider_remap_wins_over_context_that_would_be_stale() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            let mut anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();

            // Content where the context neighborhood is entirely gone —
            // context relocation alone would be Stale.
            let updated: Vec<String> = original
                .iter()
                .enumerate()
                .filter(|(i, _)| !(38..=44).contains(i))
                .map(|(_, l)| l.clone())
                .collect();
            let updated_refs = as_str_slice(&updated);
            assert_eq!(
                relocate_context(anchor.context.as_ref().unwrap(), &updated_refs),
                AnchorRelocation::Stale
            );

            let remap = ProviderRemap::line(12);
            let relocation = anchor
                .relocate_with_remap(&updated_refs, Some(&remap))
                .unwrap();
            assert_eq!(
                relocation,
                AnchorRelocation::Current {
                    new_start: 12,
                    span: 1
                }
            );
            assert_eq!(anchor.state(), AnchorState::Current);
        }

        #[test]
        fn provider_remap_sets_range_span_from_new_end() {
            let mut anchor =
                Anchor::range("src/catalog/service_03.ts", AnchorSide::New, 30, 35).unwrap();
            let remap = ProviderRemap::range(40, 46);
            let relocation = anchor.relocate_with_remap(&[], Some(&remap)).unwrap();
            assert_eq!(
                relocation,
                AnchorRelocation::Current {
                    new_start: 40,
                    span: 7
                }
            );
            match anchor.target().clone() {
                AnchorTarget::Range { start, end, .. } => {
                    assert_eq!(start, 40);
                    assert_eq!(end, 46);
                }
                _ => panic!("expected a range anchor"),
            }
        }

        #[test]
        fn without_a_provider_remap_context_relocation_still_applies() {
            let original = unique_file(100);
            let original_refs = as_str_slice(&original);
            let context = AnchorContext::capture(&original_refs, 41, 41, 2).unwrap();
            let mut anchor =
                Anchor::line_with_context("src/review-policy.ts", AnchorSide::New, 42, context)
                    .unwrap();

            let mut updated: Vec<String> = (1..=5).map(|n| format!("inserted_{n}")).collect();
            updated.extend(original.clone());
            let updated_refs = as_str_slice(&updated);

            let relocation = anchor.relocate_with_remap(&updated_refs, None).unwrap();
            assert_eq!(
                relocation,
                AnchorRelocation::Current {
                    new_start: 47,
                    span: 1
                }
            );
        }

        #[test]
        fn line_only_remap_on_a_range_anchor_is_rejected_not_collapsed() {
            let anchor_before =
                Anchor::range("src/catalog/service_03.ts", AnchorSide::New, 30, 35).unwrap();
            let mut anchor = anchor_before.clone();

            // A line-only remap (new_end: None) cannot express a range's
            // new span, so it must be rejected rather than silently
            // collapsing the range to a single line.
            let remap = ProviderRemap::line(40);
            let err = anchor
                .relocate_with_remap(&[], Some(&remap))
                .expect_err("a line-only remap must not apply to a range anchor");
            assert!(matches!(err, TuicrError::InvalidInput(_)));
            assert_eq!(
                anchor, anchor_before,
                "a rejected remap must not mutate the anchor"
            );
        }

        #[test]
        fn range_remap_on_a_line_anchor_is_rejected_not_truncated() {
            let anchor_before = Anchor::line("src/review-policy.ts", AnchorSide::New, 42);
            let mut anchor = anchor_before.clone();

            // A range remap carries a new_end that a line anchor has no
            // room for; it must be rejected rather than silently
            // discarding new_end.
            let remap = ProviderRemap::range(47, 52);
            let err = anchor
                .relocate_with_remap(&[], Some(&remap))
                .expect_err("a range remap must not apply to a line anchor");
            assert!(matches!(err, TuicrError::InvalidInput(_)));
            assert_eq!(
                anchor, anchor_before,
                "a rejected remap must not mutate the anchor"
            );
        }

        #[test]
        fn range_remap_with_new_end_before_new_start_is_rejected() {
            let anchor_before =
                Anchor::range("src/catalog/service_03.ts", AnchorSide::New, 30, 35).unwrap();
            let mut anchor = anchor_before.clone();

            let remap = ProviderRemap::range(40, 39);
            let err = anchor
                .relocate_with_remap(&[], Some(&remap))
                .expect_err("new_end before new_start must be rejected");
            assert!(matches!(err, TuicrError::InvalidInput(_)));
            assert_eq!(anchor, anchor_before);
        }

        #[test]
        fn provider_remap_on_review_or_file_anchor_is_rejected() {
            let review_before = Anchor::review();
            let mut review_anchor = review_before.clone();
            let review_err = review_anchor
                .relocate_with_remap(&[], Some(&ProviderRemap::line(10)))
                .expect_err("a review anchor has no line to move");
            assert!(matches!(review_err, TuicrError::InvalidInput(_)));
            assert_eq!(review_anchor, review_before);

            let file_before = Anchor::file("src/review-policy.ts");
            let mut file_anchor = file_before.clone();
            let file_err = file_anchor
                .relocate_with_remap(&[], Some(&ProviderRemap::line(10)))
                .expect_err("a file anchor has no line to move");
            assert!(matches!(file_err, TuicrError::InvalidInput(_)));
            assert_eq!(file_anchor, file_before);
        }
    }

    /// Compile-visible proof of the encapsulated public API: every
    /// lifecycle-critical field named by the contract (stable IDs,
    /// `Thread::status`, `Anchor::state`/`target`, `ThreadComment`'s id) is
    /// read only through an accessor here — there is no direct field
    /// access `thread.status`, `anchor.state`, `anchor.target`,
    /// `thread.id`, `thread.anchor`, or `comment.id` anywhere in this
    /// module, by construction of the accessors being the only public way
    /// to reach these values from outside `impl Thread`/`impl Anchor`/
    /// `impl ThreadComment`.
    mod encapsulation_tests {
        use super::*;

        #[test]
        fn thread_lifecycle_fields_are_read_only_through_accessors() {
            let root = ThreadComment::new(ThreadAuthor::human("alice"), "root");
            let root_id = root.id().clone();
            let mut thread = Thread::open(Anchor::line("f.ts", AnchorSide::New, 10), root);
            let thread_id = thread.id().clone();

            assert_eq!(thread.id(), &thread_id);
            assert_eq!(thread.status(), ThreadStatus::Open);
            assert_eq!(thread.anchor().state(), AnchorState::Current);
            assert_eq!(
                thread.anchor().target(),
                &AnchorTarget::Line {
                    path: "f.ts".to_string(),
                    side: AnchorSide::New,
                    line: 10,
                }
            );
            assert_eq!(thread.root().unwrap().id(), &root_id);

            // Lifecycle transitions are only reachable through the tested
            // methods; there is no way to set `status`/`anchor` directly.
            assert!(thread.resolve());
            assert_eq!(thread.status(), ThreadStatus::Resolved);
            assert!(thread.reopen());
            assert_eq!(thread.status(), ThreadStatus::Open);

            // Ids are stable across all of the above.
            assert_eq!(thread.id(), &thread_id);
            assert_eq!(thread.root().unwrap().id(), &root_id);
        }

        #[test]
        fn ids_expose_their_value_read_only_via_as_str() {
            let id = ThreadId::new();
            assert!(!id.as_str().is_empty());

            let other = ThreadId::new();
            assert_ne!(id.as_str(), other.as_str());

            let comment_id = CommentId::new();
            assert!(!comment_id.as_str().is_empty());
        }
    }
}
