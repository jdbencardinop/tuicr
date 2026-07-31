//! Azure DevOps `ForgeBackend` family.
//!
//! Azure DevOps ships two distinct product lines with different URL/host
//! conventions, plus a self-hosted "Server" variant:
//! - **Azure DevOps Services (cloud), current form**:
//!   `https://dev.azure.com/{org}/{project}/_git/{repo}` (HTTPS) and
//!   `git@ssh.dev.azure.com:v3/{org}/{project}/{repo}` (SSH). See
//!   <https://learn.microsoft.com/en-us/azure/devops/repos/git/clone>.
//! - **Azure DevOps Services, legacy `visualstudio.com` form**:
//!   `https://{org}.visualstudio.com/{project}/_git/{repo}` (optionally with
//!   a `/DefaultCollection` segment before the project) and
//!   `{org}@vs-ssh.visualstudio.com:v3/{org}/{project}/{repo}` (SSH). Both
//!   forms remain live and documented by Microsoft; the org lives in the
//!   hostname rather than the path.
//! - **Azure DevOps Server (on-prem/Team Foundation Server successor)**:
//!   `https://{server}/{collection}/{project}/_git/{repo}`, where
//!   `{collection}` (often literally `DefaultCollection`) is the on-prem
//!   analog of an Azure DevOps Services "organization". There is no
//!   universal hostname marker for these — unlike `dev.azure.com` or
//!   `*.visualstudio.com` — so, mirroring this codebase's existing
//!   Gitea/Forgejo self-hosted convention (`crate::forge::giteafj`), an
//!   on-prem host is only recognized once explicitly opted in via the
//!   `TUICR_AZURE_DEVOPS_HOSTS` environment variable (comma-separated exact
//!   hostnames, case-insensitive). This opt-in never fires by default, so
//!   `dev.azure.com`/`*.visualstudio.com`/GitHub/GitLab/Gitea/Forgejo
//!   detection is unaffected.
//!
//! `ForgeRepository.owner` holds every path segment between the host and
//! the repository name (or, for Server, between the host and `_git`),
//! joined by `/` — e.g. `"{org}/{project}"` for the cloud form, `"{project}"`
//! (or `"DefaultCollection/{project}"`) for the legacy form, and
//! `"{collection}/{project}"` for Server. This mirrors the GitLab nested-
//! group convention already in this codebase (`owner = "group/subgroup"`,
//! see `crate::forge::gitlab::glab`) rather than inventing a new multi-field
//! shape, so every generic `owner`/`slug()`/`display_name()` consumer in the
//! app keeps working unchanged. `ForgeRepository.host` is always the
//! *API* host: `dev.azure.com` for the cloud form (never
//! `ssh.dev.azure.com`, which is a clone-only alias) and `{org}.visualstudio.com`
//! for the legacy form (never `vs-ssh.visualstudio.com`), so a repo detected
//! via an SSH remote and the same repo detected via an HTTPS remote compare
//! equal.
//!
//! Known, accepted limitation shared with `crate::forge::gitlab::glab`'s
//! nested-group owners: `crate::slug::PrSlug`'s textual round-trip
//! (`ado:{owner}/{repo}/pr/{n}`) assumes `owner` has no internal `/` when
//! *parsing* it back (`parts.len() != 4` in `crate::slug::parse_pr`). A
//! multi-segment Azure `owner` (e.g. `"org/project"`) therefore does not
//! round-trip through `Slug::from_str`, exactly like GitLab's nested-group
//! `owner` already does not (see `technosylva/ai/synapse` in
//! `src/forge/gitlab/glab.rs`'s own tests). This is a pre-existing gap in
//! `crate::slug`, not introduced by this module; the only user-visible
//! effect is that `--repo` session filtering by textual slug
//! (`crate::persistence::storage::RepoSelector`) can miss a match for these
//! repos — session persistence itself (keyed by the structured
//! `ForgeRepository`, not the string slug) is unaffected.

pub mod auth;
pub mod backend;
pub mod client;
#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod live_tests;
pub mod models;
#[cfg(test)]
mod test_support;

use crate::error::{Result, TuicrError};
use crate::forge::traits::{ForgeRepository, PullRequestTarget};

const ON_PREM_HOSTS_ENV: &str = "TUICR_AZURE_DEVOPS_HOSTS";
const CLOUD_HOST: &str = "dev.azure.com";

/// Which Azure DevOps product line a host belongs to, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AzureHostKind {
    /// `dev.azure.com`.
    Cloud,
    /// `*.visualstudio.com`.
    Legacy,
    /// Explicitly opted in via `TUICR_AZURE_DEVOPS_HOSTS`.
    OnPrem,
    /// Not recognized as an Azure DevOps host at all.
    Unknown,
}

fn on_prem_allow_list() -> Vec<String> {
    std::env::var(ON_PREM_HOSTS_ENV)
        .ok()
        .map(|value| {
            value
                .split(',')
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn classify_host(host: &str, on_prem_hosts: &[String]) -> AzureHostKind {
    let lower = host.to_ascii_lowercase();
    if lower == CLOUD_HOST {
        return AzureHostKind::Cloud;
    }
    if lower.ends_with(".visualstudio.com") {
        return AzureHostKind::Legacy;
    }
    if on_prem_hosts.contains(&lower) {
        return AzureHostKind::OnPrem;
    }
    AzureHostKind::Unknown
}

/// Reads the opt-in allow-list from the environment on every call (cheap:
/// one env lookup) and delegates to the pure, directly-testable
/// [`classify_host`] — mirrors `giteafj::detect_self_hosted_kind`.
fn azure_host_kind(host: &str) -> AzureHostKind {
    classify_host(host, &on_prem_allow_list())
}

/// Whether `host` is recognized as any Azure DevOps host (cloud, legacy, or
/// an opted-in on-prem Server).
pub fn is_azure_devops_host(host: &str) -> bool {
    azure_host_kind(host) != AzureHostKind::Unknown
}

fn strip_scheme(value: &str) -> Option<&str> {
    value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
        .or_else(|| value.strip_prefix("ssh://"))
}

fn trim_url_suffix(value: &str) -> &str {
    value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .trim_end_matches('/')
}

fn strip_git_suffix(value: &str) -> &str {
    value.strip_suffix(".git").unwrap_or(value)
}

/// Parse a bare SCP-like remote (`user@host:path`, no `://`). Returns
/// `None` for anything containing `://` or missing the `host:path` shape.
fn parse_scp_like_remote(remote_url: &str) -> Option<(&str, &str)> {
    if remote_url.contains("://") {
        return None;
    }
    let (host_part, path) = remote_url.split_once(':')?;
    if host_part.contains('/') || path.is_empty() {
        return None;
    }
    let host = host_part
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(host_part);
    Some((host, path))
}

/// Parse the `v3/{org}/{project}/{repo}` SSH shorthand shared by the cloud
/// (`ssh.dev.azure.com`) and legacy (`vs-ssh.visualstudio.com`) SSH hosts,
/// normalizing the host to the corresponding API host in each case.
/// Returns `None` for any other SSH host (including an on-prem Server,
/// whose SSH clone form — when supported at all — uses the same `_git`
/// path shape as HTTPS, not this shorthand, and is handled by
/// [`repository_from_git_path`] instead).
fn azure_ssh_repository(host: &str, path: &str) -> Option<ForgeRepository> {
    let rest = path.strip_prefix("v3/")?;
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    // Minimum: org, project, repo.
    if parts.len() < 3 {
        return None;
    }
    let (owner_parts, repo_slice) = parts.split_at(parts.len() - 1);
    let repo = strip_git_suffix(trim_url_suffix(repo_slice[0]));
    let owner = owner_parts.join("/");

    let lower_host = host.to_ascii_lowercase();
    if lower_host == "ssh.dev.azure.com" {
        return Some(ForgeRepository::azure_devops(CLOUD_HOST, owner, repo));
    }
    if lower_host == "vs-ssh.visualstudio.com" {
        // The org is always the first path segment for this shorthand
        // (Microsoft's own clone-URL examples repeat it in both the SSH
        // user part and the path), so reconstruct the legacy API host
        // from the path rather than the (also-org-named) SSH user part.
        let org = owner_parts.first()?;
        return Some(ForgeRepository::azure_devops(
            format!("{org}.visualstudio.com"),
            owner,
            repo,
        ));
    }
    None
}

/// Parse an HTTPS/on-prem-style path containing a `_git` segment:
/// `{owner-segments...}/_git/{repo}[/...]`. `owner-segments` becomes
/// `ForgeRepository.owner` (joined by `/`); anything after `{repo}` (e.g.
/// `/pullrequest/{id}`) is ignored here — callers that care about it (PR
/// target parsing) re-derive it themselves from the same split.
fn repository_from_git_path(host: &str, path: &str) -> Option<ForgeRepository> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let git_index = parts.iter().position(|p| *p == "_git")?;
    if git_index == 0 {
        return None;
    }
    let repo = strip_git_suffix(trim_url_suffix(parts.get(git_index + 1)?));
    let owner = parts[..git_index].join("/");
    Some(ForgeRepository::azure_devops(host, owner, repo))
}

/// Parse a git remote URL into a `ForgeRepository` when its host is a
/// recognized Azure DevOps host (cloud, legacy, or opted-in on-prem
/// Server). Returns `None` for every other host, so callers can safely
/// chain this with the GitHub/GitLab/Gitea/Forgejo parsers without any
/// ordering hazard beyond "try this before GitHub's any-host catch-all".
pub fn parse_azure_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        return azure_ssh_repository(host, path);
    }

    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    if azure_host_kind(host) == AzureHostKind::Unknown {
        return None;
    }
    repository_from_git_path(host, path)
}

fn malformed_target<T>(input: &str) -> Result<T> {
    Err(TuicrError::Forge(format!(
        "Malformed Azure DevOps pull request target: `{input}`"
    )))
}

/// Parse `{org}/{project}/{repo}#{id}` (host defaults to `dev.azure.com`,
/// mirroring `crate::forge::gitlab::glab::parse_gitlab_repo_hash_target`'s
/// default-host convention for its own `owner/repo#N` shorthand). Requires
/// at least three `/`-separated segments before the repo so this can never
/// be confused with GitHub/Gitea/Forgejo's two-segment `owner/repo#N`.
fn parse_azure_repo_hash_target(target: &str) -> Option<PullRequestTarget> {
    let (repo_part, number_part) = target.split_once('#')?;
    let number = number_part.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    let parts: Vec<&str> = repo_part.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() < 3 {
        return None;
    }
    let (owner_parts, repo_slice) = parts.split_at(parts.len() - 1);
    let owner = owner_parts.join("/");
    let repository =
        ForgeRepository::azure_devops(CLOUD_HOST, owner, strip_git_suffix(repo_slice[0]));
    Some(PullRequestTarget::with_repository(
        repository, number, target,
    ))
}

/// Parse a full Azure DevOps pull request web URL:
/// `https://dev.azure.com/{org}/{project}/_git/{repo}/pullrequest/{id}`
/// (also matches the legacy and on-prem host forms, same path shape).
fn parse_azure_pr_url_target(target: &str) -> Option<PullRequestTarget> {
    let without_scheme = strip_scheme(target)?;
    let trimmed = trim_url_suffix(without_scheme);
    let (host, path) = trimmed.split_once('/')?;
    if azure_host_kind(host) == AzureHostKind::Unknown {
        return None;
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let git_index = parts.iter().position(|p| *p == "_git")?;
    if git_index == 0 {
        return None;
    }
    let repo = parts.get(git_index + 1)?;
    let pull_request_marker = parts.get(git_index + 2)?;
    if !pull_request_marker.eq_ignore_ascii_case("pullrequest") {
        return None;
    }
    let number = parts.get(git_index + 3)?.parse::<u64>().ok()?;
    if number == 0 {
        return None;
    }
    let owner = parts[..git_index].join("/");
    let repository =
        ForgeRepository::azure_devops(host, owner, strip_git_suffix(trim_url_suffix(repo)));
    Some(PullRequestTarget::with_repository(
        repository, number, target,
    ))
}

/// Parse a user-supplied Azure DevOps pull request target: a full PR URL
/// (`https://dev.azure.com/{org}/{project}/_git/{repo}/pullrequest/{id}`)
/// or the `{org}/{project}/{repo}#{id}` shorthand. Mirrors
/// `crate::forge::gitlab::glab::parse_pull_request_target_gitlab`'s shape
/// so `app::init::new_from_pr_target` can chain it the same way.
pub fn parse_pull_request_target_azure_devops(input: &str) -> Result<PullRequestTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return malformed_target(input);
    }
    if let Some(target) = parse_azure_pr_url_target(trimmed) {
        return Ok(target);
    }
    if let Some(target) = parse_azure_repo_hash_target(trimmed) {
        return Ok(target);
    }
    malformed_target(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_classify_cloud_host() {
        assert_eq!(classify_host("dev.azure.com", &[]), AzureHostKind::Cloud);
    }

    #[test]
    fn should_classify_legacy_visualstudio_host() {
        assert_eq!(
            classify_host("contoso.visualstudio.com", &[]),
            AzureHostKind::Legacy
        );
    }

    #[test]
    fn should_not_classify_on_prem_host_without_allow_list() {
        assert_eq!(
            classify_host("tfs.internal.example.com", &[]),
            AzureHostKind::Unknown
        );
    }

    #[test]
    fn should_classify_on_prem_host_in_allow_list() {
        let hosts = vec!["tfs.internal.example.com".to_string()];
        assert_eq!(
            classify_host("tfs.internal.example.com", &hosts),
            AzureHostKind::OnPrem
        );
    }

    #[test]
    fn should_not_classify_unrelated_hosts() {
        assert_eq!(classify_host("github.com", &[]), AzureHostKind::Unknown);
        assert_eq!(classify_host("gitlab.com", &[]), AzureHostKind::Unknown);
        assert_eq!(
            classify_host("gitea.example.com", &[]),
            AzureHostKind::Unknown
        );
    }

    #[test]
    fn should_parse_cloud_https_remote_url() {
        assert_eq!(
            parse_azure_remote_url("https://dev.azure.com/contoso/widgets/_git/api"),
            Some(ForgeRepository::azure_devops(
                "dev.azure.com",
                "contoso/widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_parse_cloud_https_remote_url_with_git_suffix() {
        assert_eq!(
            parse_azure_remote_url("https://dev.azure.com/contoso/widgets/_git/api.git"),
            Some(ForgeRepository::azure_devops(
                "dev.azure.com",
                "contoso/widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_parse_cloud_ssh_remote_url() {
        assert_eq!(
            parse_azure_remote_url("git@ssh.dev.azure.com:v3/contoso/widgets/api"),
            Some(ForgeRepository::azure_devops(
                "dev.azure.com",
                "contoso/widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_parse_legacy_https_remote_url_without_collection() {
        assert_eq!(
            parse_azure_remote_url("https://contoso.visualstudio.com/widgets/_git/api"),
            Some(ForgeRepository::azure_devops(
                "contoso.visualstudio.com",
                "widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_parse_legacy_https_remote_url_with_default_collection() {
        assert_eq!(
            parse_azure_remote_url(
                "https://contoso.visualstudio.com/DefaultCollection/widgets/_git/api"
            ),
            Some(ForgeRepository::azure_devops(
                "contoso.visualstudio.com",
                "DefaultCollection/widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_parse_legacy_ssh_remote_url() {
        assert_eq!(
            parse_azure_remote_url("contoso@vs-ssh.visualstudio.com:v3/contoso/widgets/api"),
            Some(ForgeRepository::azure_devops(
                "contoso.visualstudio.com",
                "contoso/widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_return_none_for_on_prem_host_without_opt_in() {
        assert_eq!(
            parse_azure_remote_url(
                "https://tfs.internal.example.com/DefaultCollection/widgets/_git/api"
            ),
            None
        );
    }

    #[test]
    fn should_parse_on_prem_host_via_pure_classifier_and_path_parser() {
        // Exercises the on-prem path directly through the pure helpers
        // (bypassing the env-var-gated public entry point, consistent with
        // how `giteafj`'s tests avoid mutating real process env — see
        // `giteafj::mod::tests::should_classify_host_in_gitea_allow_list`).
        let host = "tfs.internal.example.com";
        assert_eq!(
            classify_host(host, &[host.to_string()]),
            AzureHostKind::OnPrem
        );
        assert_eq!(
            repository_from_git_path(host, "DefaultCollection/widgets/_git/api"),
            Some(ForgeRepository::azure_devops(
                host,
                "DefaultCollection/widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_return_none_for_github_url() {
        assert_eq!(
            parse_azure_remote_url("https://github.com/owner/repo"),
            None
        );
    }

    #[test]
    fn should_return_none_for_gitlab_url() {
        assert_eq!(
            parse_azure_remote_url("https://gitlab.com/owner/repo"),
            None
        );
    }

    #[test]
    fn should_return_none_for_url_missing_git_segment() {
        assert_eq!(
            parse_azure_remote_url("https://dev.azure.com/contoso/widgets/api"),
            None
        );
    }

    #[test]
    fn should_parse_cloud_pr_url_target() {
        let target = parse_pull_request_target_azure_devops(
            "https://dev.azure.com/contoso/widgets/_git/api/pullrequest/42",
        )
        .expect("should parse");
        assert_eq!(target.number, 42);
        assert_eq!(
            target.repository,
            Some(ForgeRepository::azure_devops(
                "dev.azure.com",
                "contoso/widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_parse_pr_url_target_with_query_string() {
        let target = parse_pull_request_target_azure_devops(
            "https://dev.azure.com/contoso/widgets/_git/api/pullrequest/42?_a=files",
        )
        .expect("should parse");
        assert_eq!(target.number, 42);
    }

    #[test]
    fn should_parse_repo_hash_target() {
        let target =
            parse_pull_request_target_azure_devops("contoso/widgets/api#42").expect("should parse");
        assert_eq!(target.number, 42);
        assert_eq!(
            target.repository,
            Some(ForgeRepository::azure_devops(
                CLOUD_HOST,
                "contoso/widgets",
                "api"
            ))
        );
    }

    #[test]
    fn should_reject_malformed_target() {
        assert!(parse_pull_request_target_azure_devops("").is_err());
        assert!(parse_pull_request_target_azure_devops("not-a-target").is_err());
        assert!(parse_pull_request_target_azure_devops("contoso/widgets/api#0").is_err());
    }
}
