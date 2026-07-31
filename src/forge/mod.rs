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
