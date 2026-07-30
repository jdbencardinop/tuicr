//! Provider-neutral, host/version-explicit capability profiles.
//!
//! This module is pure data plus pure lookup: no I/O, no transport, no
//! credentials. It exists so the dry-run publication planner
//! (`crate::forge::dryrun`) — and, eventually, real Azure DevOps/Gitea/
//! Forgejo adapters — can ask "what can this exact host/version do" without
//! ever assuming a lowest-common-denominator shape across providers.
//!
//! Every profile constructor cites the evidence it is built from. See
//! `docs/findings/providers/provider-semantics.md` and
//! `docs/findings/providers/smoke-tests.md` in the companion research
//! repository, plus the reproducible live harness results in
//! `fixtures/providers/results-examples/{forgejo-16.0.1,gitea-1.24.7}.example.json`.
//! Nothing here claims a capability beyond what that evidence states —
//! unverified behavior is modeled as `Unsupported`/`None`, not guessed.

use serde::{Deserialize, Serialize};

use crate::forge::traits::ForgeKind;

/// How a provider exposes "the diff": one unified base/head pair, an
/// explicit version-pair (base/start/head SHAs that can each move
/// independently), or an iteration sequence. Mirrors the
/// `ProviderCapabilities.diff_model` vocabulary in
/// `docs/decisions/review-artifact-contract.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffModel {
    /// A single base/head SHA pair. GitHub, Gitea, and Forgejo all expose
    /// PR diffs this way.
    Unified,
    /// An explicit version triple (base/start/head) that can each advance
    /// independently of the others. GitLab's `diff_refs`.
    VersionPair,
    /// A sequence of numbered iterations, each with its own change list.
    /// Azure DevOps PR iterations.
    IterationPair,
}

/// Which side(s) of a diff a provider can anchor a comment to, and whether
/// it can express both sides "at once" in one native anchor (as opposed to
/// only ever picking a single side per comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideSupport {
    pub old: bool,
    pub new: bool,
    /// True when a single native anchor can carry independent old- and
    /// new-side positions at once (e.g. Azure DevOps' `threadContext` with
    /// both `leftFileStart`/`rightFileStart`). False when a comment always
    /// commits to exactly one side.
    pub simultaneous: bool,
}

impl SideSupport {
    pub const fn single_side_only() -> Self {
        Self {
            old: true,
            new: true,
            simultaneous: false,
        }
    }

    pub const fn simultaneous_both_sides() -> Self {
        Self {
            old: true,
            new: true,
            simultaneous: true,
        }
    }
}

/// Multi-line range-comment support. Mirrors the contract's
/// `range: none | same_side | dual_side_offsets` vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RangeSupport {
    /// No native range: a comment always anchors to exactly one line.
    None,
    /// A range within one side (start/end lines on the same old-or-new
    /// side).
    SameSide,
    /// Independent old- and new-side start/end offsets in one anchor.
    DualSideOffsets,
}

/// Whether — and how — a pending/draft review batching multiple comments
/// before one submit exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingReviewSupport {
    /// A pending/draft review object exists at all.
    pub supported: bool,
    /// Comments can be added to an *already-created* pending review via a
    /// separate call. When `false` (but `supported` is `true`), every
    /// comment for a review must be supplied at review-creation time in one
    /// shot. Live-proven divergence: Forgejo 16 supports this (its Swagger
    /// exposes `POST .../reviews/{id}/comments`); Gitea 1.24 does not (`GET`
    /// only on that route) — see
    /// `fixtures/providers/results-examples/{forgejo-16.0.1,gitea-1.24.7}.example.json`,
    /// keys `add_comment_to_review_endpoint_supported`.
    pub incremental_comments: bool,
}

impl PendingReviewSupport {
    pub const fn unsupported() -> Self {
        Self {
            supported: false,
            incremental_comments: false,
        }
    }

    pub const fn single_shot() -> Self {
        Self {
            supported: true,
            incremental_comments: false,
        }
    }

    pub const fn incremental() -> Self {
        Self {
            supported: true,
            incremental_comments: true,
        }
    }
}

/// How a provider expresses "request changes" on a review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestChangesSupport {
    /// A first-class review event/state (GitHub `REQUEST_CHANGES`, Gitea/
    /// Forgejo `REQUEST_CHANGES`).
    Native,
    /// Expressed as a reviewer vote value rather than a review state
    /// (Azure DevOps vote `-10`).
    Vote,
    /// No native state; substitutable with the given explicit behavior
    /// (GitLab: no request-changes review state, emulated by leaving an
    /// unresolved discussion thread).
    Emulated { substitute: String },
    /// No native state and no reasonable substitute is modeled.
    Unsupported,
}

/// How replying to an existing comment/thread works.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReplySupport {
    /// A dedicated reply/`in_reply_to` mechanism exists and is verified.
    Native,
    /// No verified reply mechanism; substitutable with the given explicit
    /// behavior.
    Emulated { substitute: String },
    /// No verified reply mechanism and no substitute is modeled. Live-proven
    /// for both Gitea 1.24 and Forgejo 16 stable: guessed reply routes
    /// return HTTP 405 on both (`swagger_has_dedicated_reply_endpoint:
    /// false` in the harness results) — this is *not* a Gitea/Forgejo
    /// divergence, so both stable profiles report the same value here.
    Unsupported,
}

/// Granularity at which "resolved" is tracked, if at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadResolutionLevel {
    /// No verified resolve/unresolve mechanism.
    None,
    /// A single comment carries a resolved flag.
    Comment,
    /// A discussion/note object carries a resolved flag (GitLab).
    Discussion,
    /// A first-class review-thread object carries resolution (GitHub
    /// GraphQL, Azure DevOps thread status).
    Thread,
}

/// How a provider signals that an anchor no longer matches the current
/// diff. The contract's base vocabulary (`none | original_position |
/// version_mismatch | tracking`) doesn't have a slot for Gitea/Forgejo's
/// explicit boolean review-level flag, so `ExplicitReviewFlag` is an
/// additive fifth variant rather than a misuse of `Tracking` (which implies
/// iteration-aware remapping Gitea/Forgejo do not do) or `OriginalPosition`
/// (which implies per-comment original/current position pairs Gitea/Forgejo
/// do not expose).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaleAnchorSignal {
    None,
    /// Per-comment original/current position fields (GitHub).
    OriginalPosition,
    /// Inferred by comparing the comment's version SHAs to the current MR
    /// version (GitLab).
    VersionMismatch,
    /// Iteration/tracking-criteria based remap (Azure DevOps).
    Tracking,
    /// A single explicit `stale` boolean on the review, not tied to
    /// per-comment repositioning (Gitea, Forgejo). Live-proven: both stable
    /// pins set `stale: true` after an anchor-shifting push
    /// (`stale_after_anchor_shift: true` in the harness results), while
    /// comments themselves stay bound to their original commit/position.
    ExplicitReviewFlag,
}

/// Suggested-edit support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionSupport {
    /// No verified suggestion mechanism.
    None,
    /// Markdown convention only, no dedicated API apply object.
    Markdown,
    /// A dedicated API object with apply semantics (GitLab `suggestions[]`).
    ApplyableObject,
}

/// How list endpoints paginate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaginationModel {
    /// Page-number/limit based (GitHub REST, GitLab, Gitea, Forgejo).
    PageNumber,
    /// Continuation-token based (Azure DevOps).
    ContinuationToken,
}

/// A minimal, human-readable provider version marker (e.g. `"16.0.1"`,
/// `"1.24.7"`). Deliberately not a full `semver::Version` newtype: profile
/// selection for the Gitea/Forgejo family only needs to bucket by
/// major.minor against the exact stable pins this codebase has live
/// evidence for, not do general semver range math.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderVersion(pub String);

impl ProviderVersion {
    pub fn new(version: impl Into<String>) -> Self {
        Self(version.into())
    }

    /// Parse `major.minor` from a leading numeric prefix (tolerating a
    /// trailing `+build.metadata` suffix, e.g. Forgejo's
    /// `"16.0.1+gitea-1.22.0"`). Returns `None` if no `major.minor` prefix
    /// parses as integers.
    fn major_minor(&self) -> Option<(u64, u64)> {
        let core = self.0.split('+').next().unwrap_or(&self.0);
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some((major, minor))
    }
}

/// Full host/version-explicit capability profile for one provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub kind: ForgeKind,
    /// The exact version this profile was evidenced against, when the
    /// family has version-dependent behavior (Gitea/Forgejo). `None` for
    /// GitHub/GitLab/Azure DevOps, whose profiles here are not modeled as
    /// version-dependent.
    pub version: Option<ProviderVersion>,
    pub diff_model: DiffModel,
    pub sides: SideSupport,
    pub range: RangeSupport,
    pub file_comment: bool,
    pub general_comment: bool,
    pub pending_review: PendingReviewSupport,
    pub request_changes: RequestChangesSupport,
    pub reply: ReplySupport,
    pub thread_resolution: ThreadResolutionLevel,
    pub stale_anchor: StaleAnchorSignal,
    pub suggestions: SuggestionSupport,
    /// No provider reviewed documents a general idempotency key for
    /// comment creation (`provider-semantics.md`, "Provider-neutral
    /// model"); this is `false` for every current profile. The client must
    /// persist an operation ID and reconcile before retrying.
    pub create_idempotency: bool,
    pub pagination: PaginationModel,
}

/// GitHub: native pending review, native request-changes, GraphQL-only
/// thread resolution, per-comment original/current position staleness,
/// Markdown-only suggestions. See `provider-semantics.md`'s review-lifecycle
/// table.
pub fn github() -> ProviderCapabilities {
    ProviderCapabilities {
        kind: ForgeKind::GitHub,
        version: None,
        diff_model: DiffModel::Unified,
        sides: SideSupport::single_side_only(),
        range: RangeSupport::SameSide,
        file_comment: true,
        general_comment: true,
        pending_review: PendingReviewSupport::incremental(),
        request_changes: RequestChangesSupport::Native,
        reply: ReplySupport::Native,
        thread_resolution: ThreadResolutionLevel::Thread,
        stale_anchor: StaleAnchorSignal::OriginalPosition,
        suggestions: SuggestionSupport::Markdown,
        create_idempotency: false,
        pagination: PaginationModel::PageNumber,
    }
}

/// GitLab: discussion-based comments/resolution, no verified pending-review
/// object (explicit evidence gap), no native request-changes state
/// (emulated via an unresolved discussion), applyable suggestions.
pub fn gitlab() -> ProviderCapabilities {
    ProviderCapabilities {
        kind: ForgeKind::GitLab,
        version: None,
        diff_model: DiffModel::VersionPair,
        sides: SideSupport::single_side_only(),
        range: RangeSupport::SameSide,
        file_comment: true,
        general_comment: true,
        // "Not verified as public REST object" per provider-semantics.md.
        pending_review: PendingReviewSupport::unsupported(),
        request_changes: RequestChangesSupport::Emulated {
            substitute: "open an unresolved discussion thread requesting changes; GitLab has no \
                         native request-changes review state"
                .to_string(),
        },
        reply: ReplySupport::Native,
        thread_resolution: ThreadResolutionLevel::Discussion,
        stale_anchor: StaleAnchorSignal::VersionMismatch,
        suggestions: SuggestionSupport::ApplyableObject,
        create_idempotency: false,
        pagination: PaginationModel::PageNumber,
    }
}

/// Azure DevOps: iteration-based diffs, simultaneous dual-side thread
/// context, vote-based approve/request-changes, thread-level resolution,
/// iteration/tracking-based staleness. No pending-review or suggestion
/// object was verified.
pub fn azure_devops() -> ProviderCapabilities {
    ProviderCapabilities {
        kind: ForgeKind::AzureDevOps,
        version: None,
        diff_model: DiffModel::IterationPair,
        sides: SideSupport::simultaneous_both_sides(),
        range: RangeSupport::DualSideOffsets,
        file_comment: true,
        general_comment: true,
        pending_review: PendingReviewSupport::unsupported(),
        request_changes: RequestChangesSupport::Vote,
        reply: ReplySupport::Native,
        thread_resolution: ThreadResolutionLevel::Thread,
        stale_anchor: StaleAnchorSignal::Tracking,
        suggestions: SuggestionSupport::None,
        create_idempotency: false,
        pagination: PaginationModel::ContinuationToken,
    }
}

/// Gitea 1.24 (live-evidenced against the `1.24.7` stable pin via
/// `fixtures/providers/gitea/run.sh`): native pending review created in one
/// shot only (no incremental add-comment route), silently drops
/// `extra_lines_count` range data rather than rejecting it (modeled as
/// `RangeSupport::None`, not a lossy `SameSide`), native request-changes,
/// no verified reply/resolve route on this stable pin, explicit
/// review-level stale flag, general comments only via a separate issue
/// comment (not the review anchor itself — still modeled `true` since the
/// mechanism exists, just outside the review object).
pub fn gitea_1_24() -> ProviderCapabilities {
    ProviderCapabilities {
        kind: ForgeKind::Gitea,
        version: Some(ProviderVersion::new("1.24.7")),
        diff_model: DiffModel::Unified,
        sides: SideSupport::single_side_only(),
        range: RangeSupport::None,
        file_comment: true,
        general_comment: true,
        pending_review: PendingReviewSupport::single_shot(),
        request_changes: RequestChangesSupport::Native,
        reply: ReplySupport::Unsupported,
        thread_resolution: ThreadResolutionLevel::None,
        stale_anchor: StaleAnchorSignal::ExplicitReviewFlag,
        suggestions: SuggestionSupport::None,
        create_idempotency: false,
        pagination: PaginationModel::PageNumber,
    }
}

/// Forgejo 16 (live-evidenced against the `16.0.1+gitea-1.22.0` stable pin
/// via `fixtures/providers/forgejo/run.sh`): same shared surface as Gitea
/// 1.24 except it accepts `extra_lines_count` range data (modeled as
/// `RangeSupport::SameSide`) and supports adding comments to an
/// already-created pending review.
pub fn forgejo_16() -> ProviderCapabilities {
    ProviderCapabilities {
        kind: ForgeKind::Forgejo,
        version: Some(ProviderVersion::new("16.0.1")),
        range: RangeSupport::SameSide,
        pending_review: PendingReviewSupport::incremental(),
        ..gitea_1_24_with_kind(ForgeKind::Forgejo)
    }
}

/// Shared skeleton for the Gitea/Forgejo family, parameterized only by
/// `kind` so `forgejo_16` can start from it via functional-update syntax
/// without duplicating every unrelated field.
fn gitea_1_24_with_kind(kind: ForgeKind) -> ProviderCapabilities {
    ProviderCapabilities {
        kind,
        ..gitea_1_24()
    }
}

/// Look up the evidence-backed capability profile for `kind`.
///
/// `version` is only consulted for the Gitea/Forgejo family, since GitHub/
/// GitLab/Azure DevOps profiles here are not modeled as version-dependent.
/// `None` picks this codebase's single evidence-backed default for that
/// kind (Gitea `1.24.7`, Forgejo `16.0.1`). A `Some` version must bucket
/// into the evidenced release line for that provider — Gitea buckets on
/// `major.minor` (only `1.24.x` is evidenced), while Forgejo buckets on
/// `major` only, since the evidence docs describe the whole stable "16"
/// line rather than a single `16.0` patch pin. This module never
/// extrapolates capabilities for an untested Gitea/Forgejo release, per
/// `provider-semantics.md`: "Never assume every Forgejo extension exists in
/// Gitea" (and, symmetrically, never assume a tested version's behavior
/// carries to an untested one either).
pub fn capabilities_for(
    kind: ForgeKind,
    version: Option<&ProviderVersion>,
) -> crate::error::Result<ProviderCapabilities> {
    use crate::error::TuicrError;

    match kind {
        ForgeKind::GitHub => Ok(github()),
        ForgeKind::GitLab => Ok(gitlab()),
        ForgeKind::AzureDevOps => Ok(azure_devops()),
        ForgeKind::Gitea => match version {
            None => Ok(gitea_1_24()),
            Some(v) if v.major_minor() == ProviderVersion::new("1.24.7").major_minor() => {
                Ok(gitea_1_24())
            }
            Some(v) => Err(TuicrError::UnsupportedOperation(format!(
                "no evidence-backed capability profile for gitea version {:?}; only 1.24.x is \
                 live-evidenced (see fixtures/providers/results-examples/gitea-1.24.7.example.json)",
                v.0
            ))),
        },
        ForgeKind::Forgejo => match version {
            None => Ok(forgejo_16()),
            // Forgejo's evidence is scoped to the stable "16" release line
            // (see provider-semantics.md / smoke-tests.md: "targeting
            // stable 16"), not a single major.minor patch pin, so bucket on
            // major version only.
            Some(v) if v.major_minor().map(|(major, _)| major) == Some(16) => Ok(forgejo_16()),
            Some(v) => Err(TuicrError::UnsupportedOperation(format!(
                "no evidence-backed capability profile for forgejo version {:?}; only 16.x is \
                 live-evidenced (see fixtures/providers/results-examples/forgejo-16.0.1.example.json)",
                v.0
            ))),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_report_github_native_pending_review_and_request_changes() {
        let caps = github();
        assert_eq!(caps.pending_review, PendingReviewSupport::incremental());
        assert_eq!(caps.request_changes, RequestChangesSupport::Native);
        assert_eq!(caps.thread_resolution, ThreadResolutionLevel::Thread);
    }

    #[test]
    fn should_emulate_gitlab_request_changes_via_unresolved_discussion() {
        let caps = gitlab();
        match caps.request_changes {
            RequestChangesSupport::Emulated { substitute } => {
                assert!(substitute.contains("unresolved"));
            }
            other => panic!("expected Emulated, got {other:?}"),
        }
        assert_eq!(caps.pending_review, PendingReviewSupport::unsupported());
    }

    #[test]
    fn should_report_azure_devops_vote_based_request_changes_and_dual_side_range() {
        let caps = azure_devops();
        assert_eq!(caps.request_changes, RequestChangesSupport::Vote);
        assert_eq!(caps.range, RangeSupport::DualSideOffsets);
        assert!(caps.sides.simultaneous);
    }

    #[test]
    fn should_diverge_gitea_and_forgejo_range_support() {
        let gitea = gitea_1_24();
        let forgejo = forgejo_16();
        assert_eq!(gitea.range, RangeSupport::None);
        assert_eq!(forgejo.range, RangeSupport::SameSide);
    }

    #[test]
    fn should_diverge_gitea_and_forgejo_incremental_pending_review_comments() {
        let gitea = gitea_1_24();
        let forgejo = forgejo_16();
        assert!(!gitea.pending_review.incremental_comments);
        assert!(forgejo.pending_review.incremental_comments);
        assert!(gitea.pending_review.supported && forgejo.pending_review.supported);
    }

    #[test]
    fn should_agree_gitea_and_forgejo_reply_and_resolve_are_unverified_on_stable() {
        let gitea = gitea_1_24();
        let forgejo = forgejo_16();
        assert_eq!(gitea.reply, ReplySupport::Unsupported);
        assert_eq!(forgejo.reply, ReplySupport::Unsupported);
        assert_eq!(gitea.thread_resolution, ThreadResolutionLevel::None);
        assert_eq!(forgejo.thread_resolution, ThreadResolutionLevel::None);
    }

    #[test]
    fn should_look_up_default_gitea_and_forgejo_profiles_without_version() {
        assert_eq!(
            capabilities_for(ForgeKind::Gitea, None).unwrap(),
            gitea_1_24()
        );
        assert_eq!(
            capabilities_for(ForgeKind::Forgejo, None).unwrap(),
            forgejo_16()
        );
    }

    #[test]
    fn should_look_up_gitea_and_forgejo_profiles_by_matching_major_minor_version() {
        let gitea_version = ProviderVersion::new("1.24.9");
        assert_eq!(
            capabilities_for(ForgeKind::Gitea, Some(&gitea_version)).unwrap(),
            gitea_1_24()
        );

        let forgejo_version = ProviderVersion::new("16.1.0+gitea-1.22.0");
        assert_eq!(
            capabilities_for(ForgeKind::Forgejo, Some(&forgejo_version)).unwrap(),
            forgejo_16()
        );
    }

    #[test]
    fn should_reject_unevidenced_gitea_version() {
        let version = ProviderVersion::new("1.23.0");
        let result = capabilities_for(ForgeKind::Gitea, Some(&version));
        assert!(result.is_err(), "1.23 has no live evidence in this repo");
    }

    #[test]
    fn should_reject_unevidenced_forgejo_version() {
        let version = ProviderVersion::new("15.0.0");
        let result = capabilities_for(ForgeKind::Forgejo, Some(&version));
        assert!(result.is_err(), "15.x has no live evidence in this repo");
    }

    #[test]
    fn should_look_up_github_gitlab_azure_profiles_ignoring_any_version() {
        let ignored = ProviderVersion::new("does-not-matter");
        assert_eq!(
            capabilities_for(ForgeKind::GitHub, Some(&ignored)).unwrap(),
            github()
        );
        assert_eq!(
            capabilities_for(ForgeKind::GitLab, Some(&ignored)).unwrap(),
            gitlab()
        );
        assert_eq!(
            capabilities_for(ForgeKind::AzureDevOps, Some(&ignored)).unwrap(),
            azure_devops()
        );
    }

    #[test]
    fn should_round_trip_capabilities_json() {
        for caps in [
            github(),
            gitlab(),
            azure_devops(),
            gitea_1_24(),
            forgejo_16(),
        ] {
            let json = serde_json::to_string(&caps).expect("serialize");
            let restored: ProviderCapabilities = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(caps, restored);
        }
    }
}
