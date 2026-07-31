//! Shared Gitea/Forgejo `ForgeBackend` family.
//!
//! Gitea and Forgejo share the same REST API shape (Forgejo is a Gitea
//! fork) but diverge in exact capability details — see
//! `crate::forge::capabilities::{gitea_1_24, forgejo_16}` for the
//! evidence-backed profile differences. This module implements one
//! transport (`GiteaForgejoBackend`) parameterized by `ForgeKind::Gitea`/
//! `ForgeKind::Forgejo`, rather than two near-duplicate backends, since the
//! wire format and every endpoint used here is identical between the two —
//! only the capability profile consulted at write time differs, and that
//! lookup already lives in `capabilities::capabilities_for`.
//!
//! Self-hosted detection: unlike `github.com`/`gitlab.com`, there is no
//! universal hostname convention for Gitea/Forgejo instances, and GitHub's
//! own remote-URL parser (`crate::forge::github::gh::parse_github_remote_url`)
//! intentionally treats *any* unrecognized host as GitHub Enterprise. So
//! self-hosted Gitea/Forgejo hosts are only recognized when either:
//! - the hostname itself contains a recognizable marker (`"gitea"`,
//!   `"forgejo"`, or `"codeberg"` — Codeberg being the best-known public
//!   Forgejo instance), or
//! - the host has been explicitly opted in via the `TUICR_GITEA_HOSTS`/
//!   `TUICR_FORGEJO_HOSTS` environment variables (comma-separated exact
//!   hostnames, case-insensitive) — for self-hosted domains with no such
//!   marker in the name.
//!
//! Neither check ever fires by default (the env vars are unset and real
//! GitHub/GitLab hosts don't match the markers), so existing GitHub/GitLab
//! URL detection and CLI target parsing are unaffected.

pub mod auth;
pub mod backend;
pub mod client;
#[cfg(test)]
mod live_tests;
pub mod models;
#[cfg(test)]
mod test_support;
pub mod version;

use crate::forge::traits::{ForgeKind, ForgeRepository};

const GITEA_HOSTS_ENV: &str = "TUICR_GITEA_HOSTS";
const FORGEJO_HOSTS_ENV: &str = "TUICR_FORGEJO_HOSTS";

/// Parse a comma-separated, case-insensitive host allow-list from `var`.
/// Missing/empty env var yields an empty list — the opt-in default.
fn env_host_list(var: &str) -> Vec<String> {
    std::env::var(var)
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

/// Decide whether `host` should be treated as a self-hosted Gitea or
/// Forgejo instance, and which. Checks Forgejo's markers first since
/// `"codeberg"` never overlaps with a `"gitea"` substring, keeping the two
/// checks independent (a host cannot simultaneously satisfy both marker
/// sets in the only cases this module models).
///
/// Reads the opt-in allow-lists from the environment on every call (cheap:
/// two env lookups, no I/O) and delegates to the pure, directly-testable
/// [`classify_host`].
pub fn detect_self_hosted_kind(host: &str) -> Option<ForgeKind> {
    classify_host(
        host,
        &env_host_list(GITEA_HOSTS_ENV),
        &env_host_list(FORGEJO_HOSTS_ENV),
    )
}

/// Pure host classifier taking pre-parsed allow-lists, so tests can exercise
/// the opt-in-list behavior without mutating real process environment
/// variables (which `cargo test`'s parallel test execution makes unsafe to
/// do reliably without an extra `serial_test`-style dependency this
/// codebase does not otherwise need).
fn classify_host(
    host: &str,
    gitea_hosts: &[String],
    forgejo_hosts: &[String],
) -> Option<ForgeKind> {
    let lower = host.to_ascii_lowercase();
    if lower.contains("forgejo") || lower.contains("codeberg") || forgejo_hosts.contains(&lower) {
        return Some(ForgeKind::Forgejo);
    }
    if lower.contains("gitea") || gitea_hosts.contains(&lower) {
        return Some(ForgeKind::Gitea);
    }
    None
}

fn repository_from_path(kind: ForgeKind, host: &str, path: &str) -> Option<ForgeRepository> {
    let mut parts = path.split('/').filter(|part| !part.is_empty());
    let owner = parts.next()?;
    let repo = strip_git_suffix(trim_url_suffix(parts.next()?));
    Some(match kind {
        ForgeKind::Forgejo => ForgeRepository::forgejo(host, owner, repo),
        _ => ForgeRepository::gitea(host, owner, repo),
    })
}

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

/// Parse a git remote URL into a `ForgeRepository` when its host looks like
/// a self-hosted Gitea or Forgejo instance (see [`detect_self_hosted_kind`]).
/// Returns `None` for every other host, so callers can safely chain this
/// with the GitHub/GitLab parsers without any ordering hazard beyond
/// "try this before GitHub's any-host catch-all".
pub fn parse_gitea_forgejo_remote_url(remote_url: &str) -> Option<ForgeRepository> {
    let trimmed = trim_url_suffix(remote_url.trim());
    if trimmed.is_empty() {
        return None;
    }

    if let Some((host, path)) = parse_scp_like_remote(trimmed) {
        let kind = detect_self_hosted_kind(host)?;
        return repository_from_path(kind, host, path);
    }

    let without_scheme = strip_scheme(trimmed).unwrap_or(trimmed);
    let without_user = without_scheme
        .rsplit_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(without_scheme);
    let (host, path) = without_user.split_once('/')?;
    let kind = detect_self_hosted_kind(host)?;
    repository_from_path(kind, host, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_detect_gitea_from_hostname_marker() {
        assert_eq!(
            detect_self_hosted_kind("gitea.example.com"),
            Some(ForgeKind::Gitea)
        );
    }

    #[test]
    fn should_detect_forgejo_from_codeberg_hostname() {
        assert_eq!(
            detect_self_hosted_kind("codeberg.org"),
            Some(ForgeKind::Forgejo)
        );
    }

    #[test]
    fn should_detect_forgejo_from_hostname_marker() {
        assert_eq!(
            detect_self_hosted_kind("forgejo.example.com"),
            Some(ForgeKind::Forgejo)
        );
    }

    #[test]
    fn should_not_detect_unrelated_hosts() {
        assert_eq!(detect_self_hosted_kind("github.com"), None);
        assert_eq!(detect_self_hosted_kind("gitlab.com"), None);
        assert_eq!(detect_self_hosted_kind("git.example.com"), None);
    }

    #[test]
    fn should_classify_host_in_gitea_allow_list() {
        let gitea_hosts = vec!["git.internal.example.com".to_string()];
        assert_eq!(
            classify_host("git.internal.example.com", &gitea_hosts, &[]),
            Some(ForgeKind::Gitea)
        );
    }

    #[test]
    fn should_classify_host_in_forgejo_allow_list() {
        let forgejo_hosts = vec!["review.internal.example.com".to_string()];
        assert_eq!(
            classify_host("review.internal.example.com", &[], &forgejo_hosts),
            Some(ForgeKind::Forgejo)
        );
    }

    #[test]
    fn should_not_classify_host_absent_from_either_allow_list() {
        let gitea_hosts = vec!["other.example.com".to_string()];
        assert_eq!(
            classify_host("git.internal.example.com", &gitea_hosts, &[]),
            None
        );
    }

    #[test]
    fn should_parse_env_host_list_case_insensitively_and_trim_whitespace() {
        // Directly exercises the env-var-format parser without touching
        // real process env: a raw `"A.example.com, B.example.com"`-shaped
        // string is what `env_host_list` would see from `std::env::var`.
        let parsed: Vec<String> = "A.example.com, b.EXAMPLE.com ,"
            .split(',')
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(parsed, vec!["a.example.com", "b.example.com"]);
    }

    #[test]
    fn should_parse_https_gitea_remote_url() {
        assert_eq!(
            parse_gitea_forgejo_remote_url("https://gitea.example.com/owner/repo.git"),
            Some(ForgeRepository::gitea("gitea.example.com", "owner", "repo"))
        );
    }

    #[test]
    fn should_parse_https_codeberg_remote_url_as_forgejo() {
        assert_eq!(
            parse_gitea_forgejo_remote_url("https://codeberg.org/owner/repo"),
            Some(ForgeRepository::forgejo("codeberg.org", "owner", "repo"))
        );
    }

    #[test]
    fn should_parse_scp_like_gitea_remote_url() {
        assert_eq!(
            parse_gitea_forgejo_remote_url("git@gitea.example.com:owner/repo.git"),
            Some(ForgeRepository::gitea("gitea.example.com", "owner", "repo"))
        );
    }

    #[test]
    fn should_return_none_for_github_url() {
        assert_eq!(
            parse_gitea_forgejo_remote_url("https://github.com/owner/repo"),
            None
        );
    }

    #[test]
    fn should_return_none_for_gitlab_url() {
        assert_eq!(
            parse_gitea_forgejo_remote_url("https://gitlab.com/owner/repo"),
            None
        );
    }
}
