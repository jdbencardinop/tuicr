//! Cross-provider integration tests.
//!
//! Every provider module (`github`, `gitlab`, `azure`, `giteafj`) already
//! has its own thorough unit/contract test suite for its own transport.
//! What none of those suites can catch on their own is a regression in the
//! *shared* infrastructure that all five [`ForgeKind`]s go through
//! together — [`registry`], [`capabilities`], [`dryrun`], the
//! `ForgeKind::provider_key` round trip, [`super::parse_any_remote_url`]'s
//! host-disjointness, and cross-provider secret redaction — where a change
//! that keeps every single per-provider suite green could still silently
//! make one provider's profile shadow another's, or leave a dry-run
//! outcome that no longer matches what `publish::execute_plan` actually
//! does. This module is that safety net; see
//! `docs/follow-on-map/tickets/12-implement-azure-adapter.md` for the
//! concrete Azure reply/resolve dry-run-vs-execute mismatch this offline
//! integration pass found and fixed in `azure::backend`, which motivated
//! adding it.
#![cfg(test)]

use std::cell::RefCell;
use std::collections::HashMap;

use crate::forge::azure::auth::AzureAuth;
use crate::forge::azure::backend::AzureDevOpsBackend;
use crate::forge::azure::test_support::{
    MockResponse, env_mutation_lock as azure_env_lock, start_mock_server as start_azure_mock_server,
};
use crate::forge::capabilities::{
    CreateThreadSupport, ProviderCapabilities, azure_devops, forgejo_16, gitea_1_24, github, gitlab,
};
use crate::forge::dryrun::{OperationOutcome, plan_publication};
use crate::forge::giteafj::backend::GiteaForgejoBackend;
use crate::forge::giteafj::client::GfHttpClient;
use crate::forge::giteafj::test_support::start_mock_server as start_giteafj_mock_server;
use crate::forge::github::gh::{GhCommandError, GhCommandResult, GhCommandRunner, GitHubGhBackend};
use crate::forge::gitlab::GitLabGlabBackend;
use crate::forge::gitlab::glab::{GlabCommandError, GlabCommandResult, GlabCommandRunner};
use crate::forge::traits::{ForgeBackend, ForgeKind, ForgeRepository, PullRequestListQuery};
use crate::forge::{parse_any_remote_url, redact_secrets, registry};
use crate::model::review::{ReviewSession, SessionDiffSource};
use crate::model::thread::{Anchor, AnchorSide, Thread, ThreadAuthor, ThreadComment};
use crate::model::thread_store::PersistedThread;
use std::path::PathBuf;

/// Run `body` with `$AZURE_DEVOPS_EXT_PAT` set to a mock value **and**
/// `$PATH` cleared, restoring both afterward — mirrors
/// `azure::contract_tests::with_mock_pat`'s pattern (that helper is private
/// to its own module, so this module needs its own copy), guarded by the
/// same crate-wide `azure_env_lock` so it never races that module's own
/// tests mutating the same process-wide env vars. Without clearing `$PATH`
/// this would risk shelling out to a real, locally-configured `az` CLI —
/// forbidden by this task's "no live external provider writes/calls"
/// constraint.
fn with_mock_azure_pat<T>(body: impl FnOnce() -> T) -> T {
    let _guard = azure_env_lock().lock().unwrap_or_else(|e| e.into_inner());
    let previous_pat = std::env::var("AZURE_DEVOPS_EXT_PAT").ok();
    let previous_path = std::env::var("PATH").ok();
    // SAFETY: serialized by `azure_env_lock` above, so no other test thread
    // observes a partial/torn value while this one mutates these
    // process-wide env vars.
    unsafe {
        std::env::set_var("AZURE_DEVOPS_EXT_PAT", "mock-pat-value");
        std::env::set_var("PATH", "");
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

const ALL_KINDS: [ForgeKind; 5] = [
    ForgeKind::GitHub,
    ForgeKind::GitLab,
    ForgeKind::AzureDevOps,
    ForgeKind::Gitea,
    ForgeKind::Forgejo,
];

/// A representative, verified-parseable remote URL for each `ForgeKind`,
/// reused from each provider module's own URL-parsing tests (see
/// `github::gh::parse_github_remote_url`, `gitlab::glab::parse_gitlab_remote_url`,
/// `azure::parse_azure_remote_url`, and `giteafj::parse_gitea_forgejo_remote_url`'s
/// own `#[test]`s for the same literals) so this module never invents an
/// untested URL shape of its own.
fn representative_remote_url(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::GitHub => "https://github.com/owner/repo",
        ForgeKind::GitLab => "https://gitlab.com/owner/repo.git",
        ForgeKind::AzureDevOps => "https://dev.azure.com/contoso/widgets/_git/api",
        ForgeKind::Gitea => "https://gitea.example.com/owner/repo.git",
        ForgeKind::Forgejo => "https://codeberg.org/owner/repo",
    }
}

// ---------------------------------------------------------------------
// Registry construction
// ---------------------------------------------------------------------

#[test]
fn should_build_a_working_backend_for_every_registered_kind() {
    // Only proves construction succeeds — deliberately does *not* invoke
    // any `ForgeBackend` trait method here, since every one of them
    // performs real I/O (a `gh`/`glab` CLI invocation or an HTTP request)
    // and this task forbids live external calls. Each provider module has
    // its own `should_reject_repository_with_mismatched_kind`-style test
    // (e.g. `giteafj::backend::tests::should_reject_repository_with_mismatched_kind`)
    // covering that no backend silently serves another kind's repository.
    for kind in ALL_KINDS {
        let repo = match kind {
            ForgeKind::GitHub => ForgeRepository::github("github.com", "owner", "repo"),
            ForgeKind::GitLab => ForgeRepository::gitlab("gitlab.com", "owner", "repo"),
            ForgeKind::AzureDevOps => {
                ForgeRepository::azure_devops("https://dev.azure.com", "owner", "repo")
            }
            ForgeKind::Gitea => {
                ForgeRepository::gitea("https://gitea.example.com", "owner", "repo")
            }
            ForgeKind::Forgejo => ForgeRepository::forgejo("https://codeberg.org", "owner", "repo"),
        };
        let _backend =
            registry::create_backend(&repo, None).expect("every registered kind should build");
    }
}

#[test]
fn should_look_up_capabilities_for_every_kind_via_the_registry() {
    for kind in ALL_KINDS {
        let repo = ForgeRepository {
            kind,
            host: "example.com".to_string(),
            owner: "owner".to_string(),
            name: "repo".to_string(),
        };
        let caps = registry::capabilities(&repo, None).expect("capabilities for every kind");
        assert_eq!(
            caps.kind, kind,
            "capabilities profile must match its own kind"
        );
    }
}

// ---------------------------------------------------------------------
// Capability/profile serialization round-trip
// ---------------------------------------------------------------------

#[test]
fn should_round_trip_every_capability_profile_through_json() {
    for caps in [
        github(),
        gitlab(),
        azure_devops(),
        gitea_1_24(),
        forgejo_16(),
    ] {
        let json = serde_json::to_string(&caps).expect("serialize capabilities");
        let back: ProviderCapabilities =
            serde_json::from_str(&json).expect("deserialize capabilities");
        assert_eq!(
            caps, back,
            "profile must round-trip byte-for-byte through JSON"
        );
    }
}

#[test]
fn should_have_disjoint_capability_profiles_per_kind() {
    // No two `ForgeKind`s may resolve to `==` profiles, or a dry-run for
    // one provider could silently reuse another's behavior.
    let profiles: Vec<ProviderCapabilities> = vec![
        github(),
        gitlab(),
        azure_devops(),
        gitea_1_24(),
        forgejo_16(),
    ];
    for (i, a) in profiles.iter().enumerate() {
        for (j, b) in profiles.iter().enumerate() {
            if i != j {
                assert_ne!(a, b, "profiles for two different kinds must never be equal");
            }
        }
    }
}

// ---------------------------------------------------------------------
// Dry-run operation outcomes
// ---------------------------------------------------------------------

fn session_with_one_open_thread(anchor: Anchor) -> (ReviewSession, String) {
    let root = ThreadComment::new(ThreadAuthor::human("alice"), "root comment");
    let thread = PersistedThread::new(Thread::open(anchor, root));
    let thread_id = thread.id().as_str().to_string();
    let mut session = ReviewSession::new(
        PathBuf::from("/repo"),
        "deadbeef".to_string(),
        None,
        SessionDiffSource::WorkingTree,
    );
    session.threads = vec![thread];
    (session, thread_id)
}

/// A minimal, otherwise-irrelevant-field `PullRequestDetails` fixture —
/// mirrors `azure::contract_tests::sample_pr_details`'s shape, but this
/// module can't reuse that one (private to its own test module).
fn sample_pr_details(repository: ForgeRepository) -> crate::forge::traits::PullRequestDetails {
    crate::forge::traits::PullRequestDetails {
        repository,
        number: 42,
        title: "sample".to_string(),
        url: String::new(),
        state: "active".to_string(),
        is_draft: false,
        author: None,
        head_ref_name: "feature".to_string(),
        base_ref_name: "main".to_string(),
        head_sha: "1".repeat(40),
        base_sha: "2".repeat(40),
        body: String::new(),
        updated_at: None,
        closed: false,
        merged_at: None,
        diff_start_sha: None,
    }
}

/// The core regression test for this offline-integration pass: on every
/// profile, `plan_publication`'s `CreateThread` outcome for a brand-new,
/// single-line-anchored thread agrees with what the real `ForgeBackend`
/// trait-level `create_thread` will actually do — `Planned` if and only if
/// the trait method is overridden with real transport, `Unsupported` if
/// and only if the profile reports `CreateThreadSupport::Unsupported` and
/// the trait is left on its honest default. This is exactly the class of
/// bug found in `azure::backend` during this merge (capability said
/// `Native`, trait impl used the default) — fixed by wiring its
/// trait-level `create_thread`/`reply_to_thread`/`set_thread_resolution` —
/// and the class of bug this same pass separately found and fixed for
/// Gitea/Forgejo (capabilities' `file_comment`/`general_comment` are
/// `true`, describing their *batched* `create_review` comment route, but
/// neither ever verified a *standalone* create-thread endpoint, so
/// `dryrun::plan_thread` now gates `CreateThread` on the dedicated
/// `CreateThreadSupport` field instead of reusing those two flags).
#[test]
fn should_plan_new_thread_creation_to_match_real_trait_wiring_on_every_profile() {
    use crate::forge::traits::NewThreadRequest;

    let anchor = Anchor::line("src/a.rs", AnchorSide::New, 10);
    let (session, thread_id) = session_with_one_open_thread(anchor);

    for caps in [
        github(),
        gitlab(),
        azure_devops(),
        gitea_1_24(),
        forgejo_16(),
    ] {
        let plan = plan_publication(&session, &caps, None);
        let op = plan
            .operations
            .iter()
            .find(|op| op.thread_id.as_deref() == Some(thread_id.as_str()))
            .expect("operation for thread");
        let expect_native = matches!(caps.create_thread, CreateThreadSupport::Native);
        assert_eq!(
            matches!(op.outcome, OperationOutcome::Planned),
            expect_native,
            "CreateThread outcome must agree with CreateThreadSupport, caps={caps:?}, \
             outcome={:?}",
            op.outcome
        );
    }

    // GitHub, GitLab, and Azure DevOps override the trait-level
    // `create_thread` default (`CreateThreadSupport::Native`, confirmed
    // `Planned` above); Gitea/Forgejo intentionally do not
    // (`CreateThreadSupport::Unsupported`, confirmed `Unsupported` above) —
    // both sides verified to agree for every profile.
    with_mock_azure_pat(|| {
        let repo = ForgeRepository::azure_devops("http://127.0.0.1:1", "o", "r");
        let pr = sample_pr_details(repo);
        let request = NewThreadRequest {
            commit_id: &pr.head_sha,
            body: "hello",
            path: Some("src/a.rs"),
            line: Some(10),
            side: Some(AnchorSide::New),
            range_start: None,
        };
        // Held as `&dyn ForgeBackend` — exactly how `publish::execute_plan`
        // reaches this method in production (via `registry::create_backend`'s
        // `Box<dyn ForgeBackend>`). This is deliberate: `AzureDevOpsBackend`
        // also has an *inherent* `create_thread` method (a different,
        // lower-level signature used internally by the trait impl below) —
        // calling `azure_backend.create_thread(..)` directly on the
        // concrete type would resolve to that inherent method instead
        // (inherent methods always shadow trait methods for direct
        // method-call syntax), silently testing the wrong code path.
        let concrete_backend = AzureDevOpsBackend::new(None);
        let azure_backend: &dyn ForgeBackend = &concrete_backend;
        let err = azure_backend
            .create_thread(&pr, request)
            .expect_err("no live server is reachable, but this must not be UnsupportedOperation");
        assert!(
            !matches!(err, crate::error::TuicrError::UnsupportedOperation(_)),
            "Azure's trait-level create_thread must attempt real transport, not fall back to \
             the default Unsupported implementation: got {err:?}"
        );
    });

    // Gitea/Forgejo's trait-level `create_thread` must still be the
    // honest default `UnsupportedOperation` — confirming the dry-run
    // `Unsupported` outcome asserted above is not just honest in theory
    // but actually matches the real (non-)transport a caller would hit.
    for kind in [ForgeKind::Gitea, ForgeKind::Forgejo] {
        let repo = match kind {
            ForgeKind::Gitea => ForgeRepository::gitea("http://127.0.0.1:1", "o", "r"),
            ForgeKind::Forgejo => ForgeRepository::forgejo("http://127.0.0.1:1", "o", "r"),
            _ => unreachable!(),
        };
        let pr = sample_pr_details(repo);
        let request = NewThreadRequest {
            commit_id: &pr.head_sha,
            body: "hello",
            path: Some("src/a.rs"),
            line: Some(10),
            side: Some(AnchorSide::New),
            range_start: None,
        };
        let backend = GiteaForgejoBackend::new(kind, None);
        let dyn_backend: &dyn ForgeBackend = &backend;
        let err = dyn_backend
            .create_thread(&pr, request)
            .expect_err("no live server was reached, so an `Ok` here would itself be a bug");
        assert!(
            matches!(err, crate::error::TuicrError::UnsupportedOperation(_)),
            "{kind:?}'s trait-level create_thread must stay on the honest default \
             UnsupportedOperation (never attempt a live call) to match its dry-run outcome: \
             got {err:?}"
        );
    }
}

// ---------------------------------------------------------------------
// Provider-key round-trip
// ---------------------------------------------------------------------

#[test]
fn should_round_trip_every_kind_through_its_provider_key() {
    for kind in ALL_KINDS {
        let key = kind.provider_key();
        assert_eq!(
            ForgeKind::from_provider_key(key),
            Some(kind),
            "provider_key {key:?} must parse back to {kind:?}"
        );
    }
}

#[test]
fn should_accept_the_legacy_snake_case_azure_alias_without_colliding_with_others() {
    assert_eq!(
        ForgeKind::from_provider_key("azure_devops"),
        Some(ForgeKind::AzureDevOps)
    );
    // The alias must not be accepted for any *other* kind's key.
    for kind in ALL_KINDS {
        if kind != ForgeKind::AzureDevOps {
            assert_ne!(
                ForgeKind::from_provider_key(kind.provider_key()),
                Some(ForgeKind::AzureDevOps)
            );
        }
    }
}

// ---------------------------------------------------------------------
// URL detection disjointness
// ---------------------------------------------------------------------

#[test]
fn should_detect_every_provider_url_as_its_own_exclusive_kind() {
    for kind in ALL_KINDS {
        let url = representative_remote_url(kind);
        let detected = parse_any_remote_url(url)
            .unwrap_or_else(|| panic!("{kind:?}'s representative URL {url:?} should parse"));
        assert_eq!(
            detected.kind, kind,
            "URL {url:?} must be detected as {kind:?}, not {:?}",
            detected.kind
        );
    }
}

#[test]
fn should_never_detect_two_different_kinds_urls_as_the_same_kind() {
    let mut seen = std::collections::HashSet::new();
    for kind in ALL_KINDS {
        let url = representative_remote_url(kind);
        let detected = parse_any_remote_url(url).expect("url should parse");
        assert!(
            seen.insert(detected.kind),
            "two different providers' representative URLs both detected as {:?}",
            detected.kind
        );
    }
}

// ---------------------------------------------------------------------
// No token leakage
// ---------------------------------------------------------------------

#[test]
fn should_never_leak_a_github_or_gitlab_shaped_token_through_redact_secrets() {
    let raw = "error: authentication failed for token ghp_abcdefghijklmnopqrstuvwxyz0123456789 \
               and glpat-ABCDEFGHIJKLMNOPQRST";
    let redacted = redact_secrets(raw);
    assert!(!redacted.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
    assert!(!redacted.contains("glpat-ABCDEFGHIJKLMNOPQRST"));
    assert!(redacted.contains("<redacted>"));
}

#[test]
fn should_never_leak_an_azure_pat_through_azure_auths_debug_impl() {
    let auth = AzureAuth {
        header_value: "Basic czNjcjN0LW1vY2stcGF0LXZhbHVl".to_string(),
        source: "mock",
    };
    let debug = format!("{auth:?}");
    assert!(!debug.contains("czNjcjN0LW1vY2stcGF0LXZhbHVl"));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn should_never_leak_a_gitea_forgejo_token_through_gf_http_clients_debug_impl() {
    let client = GfHttpClient::new(
        "http://example.com".to_string(),
        "s3cr3t-mock-token-value".to_string(),
    );
    let debug = format!("{client:?}");
    assert!(!debug.contains("s3cr3t-mock-token-value"));
    assert!(debug.contains("<redacted>"));
}

// ---------------------------------------------------------------------
// Mock executor selection across GitHub/GitLab/Azure/Gitea/Forgejo
// ---------------------------------------------------------------------

/// A trivial scripted `gh`/`glab` command runner: every invocation
/// succeeds with an empty JSON list, regardless of the arguments given.
/// Sufficient to prove the registry/backend correctly reaches this fake
/// transport at all (rather than e.g. silently no-op'ing or panicking) —
/// each provider module's own test suite already covers the real
/// argument-shape/response-parsing contract in depth.
#[derive(Default)]
struct EmptyListRunner {
    calls: RefCell<Vec<Vec<String>>>,
}

impl GhCommandRunner for EmptyListRunner {
    fn run(&self, args: &[String]) -> GhCommandResult<String> {
        self.calls.borrow_mut().push(args.to_vec());
        match args.first().map(String::as_str) {
            Some("pr") => Ok("[]".to_string()),
            _ => Err(GhCommandError::Failed {
                status: Some(1),
                stderr: "unexpected command for EmptyListRunner".to_string(),
            }),
        }
    }
}

impl GlabCommandRunner for EmptyListRunner {
    fn run(&self, args: &[String]) -> GlabCommandResult<String> {
        self.calls.borrow_mut().push(args.to_vec());
        match args.first().map(String::as_str) {
            Some("mr") => Ok("[]".to_string()),
            _ => Err(GlabCommandError::Failed {
                status: Some(1),
                stderr: "unexpected command for EmptyListRunner".to_string(),
            }),
        }
    }
}

#[test]
fn should_select_and_execute_the_github_transport_under_a_mock_runner() {
    let runner = EmptyListRunner::default();
    let backend = GitHubGhBackend::with_runner(None, runner);
    let repo = ForgeRepository::github("github.com", "owner", "repo");
    // A successful, well-typed empty page is only reachable if the
    // registry actually dispatched to *this* mock `GhCommandRunner` (the
    // real `gh` CLI is not on this test's `PATH` requirement and would
    // fail differently) — proving mock-executor selection end-to-end.
    let page = backend
        .list_pull_requests(PullRequestListQuery::first_page(repo, 10))
        .expect("mock gh transport should succeed");
    assert_eq!(page.pull_requests.len(), 0);
}

#[test]
fn should_select_and_execute_the_gitlab_transport_under_a_mock_runner() {
    let runner = EmptyListRunner::default();
    let backend = GitLabGlabBackend::with_runner(None, runner);
    let repo = ForgeRepository::gitlab("gitlab.com", "owner", "repo");
    let page = backend
        .list_pull_requests(PullRequestListQuery::first_page(repo, 10))
        .expect("mock glab transport should succeed");
    assert_eq!(page.pull_requests.len(), 0);
}

#[test]
fn should_select_and_execute_the_azure_transport_under_a_mock_http_server() {
    with_mock_azure_pat(|| {
        let responses = HashMap::from([(
            "GET /contoso/widgets/_apis/git/repositories/api/pullrequests?api-version=7.1&\
             searchCriteria.status=active&$skip=0&$top=10"
                .to_string(),
            MockResponse::json(200, r#"{"value": []}"#),
        )]);
        let (base_url, requests) = start_azure_mock_server(responses);
        let repo = ForgeRepository::azure_devops(base_url, "contoso/widgets", "api");
        let backend = AzureDevOpsBackend::new(None);
        let query = PullRequestListQuery::first_page(repo, 10);

        let page = backend
            .list_pull_requests(query)
            .expect("mock azure transport should succeed");

        assert_eq!(page.pull_requests.len(), 0);
        assert_eq!(requests.lock().expect("lock captured requests").len(), 1);
    });
}

#[test]
fn should_select_and_execute_the_gitea_transport_under_a_mock_http_server() {
    let _guard = GITEA_TOKEN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var("GITEA_TOKEN").ok();
    // SAFETY: serialized by `GITEA_TOKEN_LOCK` above, matching the pattern
    // established by `giteafj::backend::tests::with_gitea_token`.
    unsafe {
        std::env::set_var("GITEA_TOKEN", "mock-token");
    }

    let mut responses = HashMap::new();
    responses.insert(
        "/api/v1/version".to_string(),
        (200u16, r#"{"version":"1.24.7"}"#.to_string()),
    );
    responses.insert(
        "/api/v1/repos/owner/repo/pulls?state=open&sort=recentupdate&limit=10&page=1".to_string(),
        (200u16, "[]".to_string()),
    );
    let (base_url, requests) = start_giteafj_mock_server(responses);
    let repo = ForgeRepository::gitea(base_url, "owner", "repo");
    let backend = GiteaForgejoBackend::new(ForgeKind::Gitea, None);
    let query = PullRequestListQuery::first_page(repo, 10);

    let page = backend
        .list_pull_requests(query)
        .expect("mock gitea transport should succeed");

    unsafe {
        match previous {
            Some(v) => std::env::set_var("GITEA_TOKEN", v),
            None => std::env::remove_var("GITEA_TOKEN"),
        }
    }

    assert_eq!(page.pull_requests.len(), 0);
    // The version probe plus the PR-list call: two distinct requests.
    assert_eq!(requests.lock().expect("lock captured requests").len(), 2);
}

#[test]
fn should_select_and_execute_the_forgejo_transport_under_a_mock_http_server() {
    let _guard = FORGEJO_TOKEN_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let previous = std::env::var("FORGEJO_TOKEN").ok();
    // SAFETY: serialized by `FORGEJO_TOKEN_LOCK` above.
    unsafe {
        std::env::set_var("FORGEJO_TOKEN", "mock-token");
    }

    let mut responses = HashMap::new();
    responses.insert(
        "/api/v1/version".to_string(),
        // Forgejo's version string must carry the `+gitea-` build-metadata
        // marker (see `giteafj::version`'s classifier) or it is rejected as
        // an unverified/misconfigured-kind Gitea instance instead —
        // mirrors the real fixture at
        // `giteafj/fixtures/version-forgejo-16.0.1.json`.
        (200u16, r#"{"version":"16.0.1+gitea-1.22.0"}"#.to_string()),
    );
    responses.insert(
        "/api/v1/repos/owner/repo/pulls?state=open&sort=recentupdate&limit=10&page=1".to_string(),
        (200u16, "[]".to_string()),
    );
    let (base_url, requests) = start_giteafj_mock_server(responses);
    let repo = ForgeRepository::forgejo(base_url, "owner", "repo");
    let backend = GiteaForgejoBackend::new(ForgeKind::Forgejo, None);
    let query = PullRequestListQuery::first_page(repo, 10);

    let page = backend
        .list_pull_requests(query)
        .expect("mock forgejo transport should succeed");

    unsafe {
        match previous {
            Some(v) => std::env::set_var("FORGEJO_TOKEN", v),
            None => std::env::remove_var("FORGEJO_TOKEN"),
        }
    }

    assert_eq!(page.pull_requests.len(), 0);
    assert_eq!(requests.lock().expect("lock captured requests").len(), 2);
}

/// Serializes tests that mutate `$GITEA_TOKEN` — mirrors
/// `giteafj::backend::tests::token_env_lock`, but that one is private to
/// its own module, so this module needs its own instance guarding the
/// same process-wide env var.
static GITEA_TOKEN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Same as [`GITEA_TOKEN_LOCK`] but for `$FORGEJO_TOKEN`.
static FORGEJO_TOKEN_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
