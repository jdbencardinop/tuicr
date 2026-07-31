//! Remote forge integration.
//!
//! This module is intentionally transport-focused for the first integration
//! slice. UI and review submission code should depend on the trait shape here
//! instead of shelling out to forge-specific tools directly.
#![allow(dead_code)]

pub mod canonical;
pub mod capabilities;
pub mod context;
pub mod dryrun;
pub mod giteafj;
pub mod github;
pub mod gitlab;
pub mod pr_open;
pub mod publish;
pub mod registry;
pub mod remote_comments;
pub mod selector;
pub mod submit;
pub mod traits;

use std::path::{Path, PathBuf};

use git2::Repository;

use crate::forge::giteafj::parse_gitea_forgejo_remote_url;
use crate::forge::github::gh::parse_github_remote_url;
use crate::forge::gitlab::glab::parse_gitlab_remote_url;
use crate::forge::traits::ForgeRepository;

/// `repo_root`'s remote URLs, `origin` first, then every other remote.
fn remote_urls(repo_root: &Path) -> Vec<String> {
    let Ok(repo) = Repository::discover(repo_root) else {
        return Vec::new();
    };
    let mut all_urls: Vec<String> = Vec::new();

    if let Ok(remote) = repo.find_remote("origin")
        && let Some(url) = remote.url()
    {
        all_urls.push(url.to_string());
    }
    if let Ok(remotes) = repo.remotes() {
        for name in remotes.iter().flatten() {
            if let Ok(remote) = repo.find_remote(name)
                && let Some(url) = remote.url()
            {
                all_urls.push(url.to_string());
            }
        }
    }
    all_urls
}

/// Parse `url` as a forge remote repository.
///
/// Tries GitLab first — its parser already filters to "gitlab" hosts, so
/// trying it first won't claim GitHub Enterprise remotes — then Gitea/
/// Forgejo (also host-filtered; see
/// `crate::forge::giteafj::detect_self_hosted_kind`), then falls back to
/// GitHub, which accepts any host (covers github.com and GHE hosts whose
/// hostname does not literally contain "github"). The Gitea/Forgejo check
/// must run before the GitHub catch-all or it would never get a chance to
/// match anything.
pub fn parse_any_remote_url(url: &str) -> Option<ForgeRepository> {
    parse_gitlab_remote_url(url)
        .or_else(|| parse_gitea_forgejo_remote_url(url))
        .or_else(|| parse_github_remote_url(url))
}

/// Detect the forge repository for the local checkout at `repo_root`.
/// Returns `None` when no remote can be parsed.
pub fn detect_forge_repository(repo_root: &Path) -> Option<ForgeRepository> {
    remote_urls(repo_root)
        .iter()
        .find_map(|url| parse_any_remote_url(url))
}

/// `root`'s local checkout, but only when one of its remotes — not
/// necessarily `origin` — matches `target_repo`.
pub fn local_checkout_for_repo(root: &Path, target_repo: &ForgeRepository) -> Option<PathBuf> {
    remote_urls(root)
        .iter()
        .any(|url| parse_any_remote_url(url).as_ref() == Some(target_repo))
        .then(|| root.to_path_buf())
}

/// Known credential-shaped token prefixes GitHub/GitLab issue for personal
/// access tokens. Neither backend ever reads or constructs a token value
/// itself (`gh`/`glab` resolve credentials from their own auth state — see
/// `github::gh`/`gitlab::glab`'s `should_never_leak_environment_token_into_*`
/// structural regression tests), so in normal operation none of these ever
/// appear in a command's stderr. This is a defense-in-depth backstop for
/// the "unmatched error" fallback paths (`github::gh::map_gh_error`,
/// `gitlab::glab::map_glab_error`) that otherwise embed a CLI's raw stderr
/// verbatim: if `gh`/`glab` diagnostics ever changed to echo a credential,
/// it would still never reach a persisted session, a rendered error
/// message, or a log.
const KNOWN_TOKEN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "github_pat_",
    "glpat-",
    "glpat_",
];

fn is_token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

/// Replace every occurrence of a [`KNOWN_TOKEN_PREFIXES`] entry (at a token
/// boundary — not embedded mid-word) through the end of the run of
/// token-shaped characters that follows it with `<redacted>`. Everything
/// else, including surrounding text/whitespace/punctuation, passes through
/// unchanged.
pub(crate) fn redact_secrets(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let at_boundary = i == 0 || !is_token_char(bytes[i - 1]);
        let matched = at_boundary
            .then(|| {
                KNOWN_TOKEN_PREFIXES
                    .iter()
                    .find(|prefix| text[i..].starts_with(**prefix))
            })
            .flatten();
        if let Some(prefix) = matched {
            let mut end = i + prefix.len();
            while end < text.len() && is_token_char(bytes[end]) {
                end += 1;
            }
            out.push_str("<redacted>");
            i = end;
        } else {
            let ch = text[i..].chars().next().expect("i < text.len()");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_repo_with_origin(url: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("init repo");
        repo.remote("origin", url).expect("add origin");
        dir
    }

    #[test]
    fn should_redact_github_and_gitlab_token_prefixes_leaving_surrounding_text_intact() {
        let input = "gh: HTTP 500: token ghp_abcDEF0123456789abcdefABCDEF012345 rejected \
                     by proxy for glpat-XyZ_0123456789abcdef";
        let redacted = redact_secrets(input);
        assert!(!redacted.contains("ghp_"));
        assert!(!redacted.contains("glpat-"));
        assert_eq!(redacted.matches("<redacted>").count(), 2);
        assert!(redacted.starts_with("gh: HTTP 500: token <redacted> rejected by proxy for "));
    }

    #[test]
    fn should_not_redact_text_that_merely_contains_a_prefix_mid_word() {
        // `is_token_char` boundary check: a prefix embedded inside a larger
        // identifier (not preceded by a non-token-char boundary) is not a
        // real token occurrence and must pass through untouched.
        let input = "xghp_not_a_real_token_boundary";
        assert_eq!(redact_secrets(input), input);
    }

    #[test]
    fn should_leave_ordinary_error_text_completely_unchanged() {
        let input = "HTTP 403: Resource not accessible by integration";
        assert_eq!(redact_secrets(input), input);
    }

    #[test]
    fn detects_github_repository_from_origin() {
        let dir = init_repo_with_origin("https://github.com/agavra/tuicr");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::github("github.com", "agavra", "tuicr"))
        );
    }

    #[test]
    fn detects_self_hosted_gitea_repository_from_origin() {
        let dir = init_repo_with_origin("https://gitea.example.com/agavra/tuicr.git");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::gitea(
                "gitea.example.com",
                "agavra",
                "tuicr"
            ))
        );
    }

    #[test]
    fn detects_codeberg_repository_as_forgejo_from_origin() {
        let dir = init_repo_with_origin("https://codeberg.org/agavra/tuicr");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::forgejo("codeberg.org", "agavra", "tuicr"))
        );
    }

    #[test]
    fn still_detects_gitlab_repository_from_origin_unaffected_by_gitea_forgejo_ordering() {
        let dir = init_repo_with_origin("https://gitlab.com/agavra/tuicr");
        assert_eq!(
            detect_forge_repository(dir.path()),
            Some(ForgeRepository::gitlab("gitlab.com", "agavra", "tuicr"))
        );
    }

    #[test]
    fn local_checkout_matches_when_origin_equals_target() {
        let dir = init_repo_with_origin("https://github.com/agavra/tuicr");
        let target = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert_eq!(
            local_checkout_for_repo(dir.path(), &target),
            Some(dir.path().to_path_buf())
        );
    }

    #[test]
    fn local_checkout_rejects_mismatched_repo() {
        let dir = init_repo_with_origin("https://github.com/contributor/tuicr");
        let target = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert_eq!(local_checkout_for_repo(dir.path(), &target), None);
    }

    #[test]
    fn local_checkout_returns_none_outside_a_repo() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert_eq!(local_checkout_for_repo(dir.path(), &target), None);
    }

    #[test]
    fn local_checkout_matches_upstream_remote_in_fork_workflow() {
        let dir = init_repo_with_origin("https://github.com/contributor/tuicr");
        let repo = Repository::open(dir.path()).expect("open repo");
        repo.remote("upstream", "https://github.com/agavra/tuicr")
            .expect("add upstream");

        let target = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert_eq!(
            local_checkout_for_repo(dir.path(), &target),
            Some(dir.path().to_path_buf())
        );
    }
}
