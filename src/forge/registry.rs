//! Bounded provider registry: capability lookup plus a typed-error
//! transport factory.
//!
//! This replaces the previously closed `ForgeKind` match in
//! `crate::app::create_forge_backend`. GitHub and GitLab construct their
//! existing `gh`/`glab` transports; Gitea, Forgejo, and Azure DevOps use
//! their own HTTP transports (`crate::forge::giteafj::backend`,
//! `crate::forge::azure::backend`). Every kind currently has a working
//! transport; this factory is kept in case a future kind is registered
//! with capabilities only (see `capabilities_for`) before its transport
//! lands — such a kind would return
//! `Err(TuicrError::UnsupportedOperation(_))` here, never a panic and
//! never a silent fallback to a different kind.

use std::path::PathBuf;

use crate::error::Result;
use crate::forge::capabilities::{ProviderCapabilities, ProviderVersion, capabilities_for};
use crate::forge::traits::{ForgeBackend, ForgeKind, ForgeRepository};

/// Look up `repo.kind`'s capability profile. See
/// [`crate::forge::capabilities::capabilities_for`] for version-selection
/// rules; `version` is only consulted for the Gitea/Forgejo family.
pub fn capabilities(
    repo: &ForgeRepository,
    version: Option<&ProviderVersion>,
) -> Result<ProviderCapabilities> {
    capabilities_for(repo.kind, version)
}

/// Construct a transport for `repo`.
///
/// Returns `Ok` for every currently-registered `ForgeKind`:
/// `GitHub`/`GitLab` reuse the existing `gh`/`glab`-shelling backends
/// unchanged; `Gitea`/`Forgejo` share
/// `crate::forge::giteafj::backend::GiteaForgejoBackend`;
/// `AzureDevOps` uses `crate::forge::azure::backend::AzureDevOpsBackend`.
/// This factory must never panic or silently substitute another kind's
/// backend.
pub fn create_backend(
    repo: &ForgeRepository,
    local_checkout: Option<PathBuf>,
) -> Result<Box<dyn ForgeBackend>> {
    match repo.kind {
        ForgeKind::GitHub => {
            use crate::forge::github::gh::GitHubGhBackend;
            Ok(Box::new(
                GitHubGhBackend::new(Some(repo.clone())).with_local_checkout(local_checkout),
            ))
        }
        ForgeKind::GitLab => {
            use crate::forge::gitlab::GitLabGlabBackend;
            Ok(Box::new(
                GitLabGlabBackend::new(Some(repo.clone())).with_local_checkout(local_checkout),
            ))
        }
        ForgeKind::Gitea | ForgeKind::Forgejo => {
            use crate::forge::giteafj::backend::GiteaForgejoBackend;
            Ok(Box::new(
                GiteaForgejoBackend::new(repo.kind, Some(repo.clone()))
                    .with_local_checkout(local_checkout),
            ))
        }
        ForgeKind::AzureDevOps => {
            use crate::forge::azure::backend::AzureDevOpsBackend;
            Ok(Box::new(
                AzureDevOpsBackend::new(Some(repo.clone())).with_local_checkout(local_checkout),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_build_github_backend() {
        let repo = ForgeRepository::github("github.com", "agavra", "tuicr");
        assert!(create_backend(&repo, None).is_ok());
    }

    #[test]
    fn should_build_gitlab_backend() {
        let repo = ForgeRepository::gitlab("gitlab.com", "group", "project");
        assert!(create_backend(&repo, None).is_ok());
    }

    #[test]
    fn should_build_azure_devops_backend() {
        let repo = ForgeRepository::azure_devops("dev.azure.com", "org", "project");
        assert!(create_backend(&repo, None).is_ok());
    }

    #[test]
    fn should_build_gitea_backend() {
        let repo = ForgeRepository::gitea("gitea.example.com", "owner", "repo");
        assert!(create_backend(&repo, None).is_ok());
    }

    #[test]
    fn should_build_forgejo_backend() {
        let repo = ForgeRepository::forgejo("codeberg.org", "owner", "repo");
        assert!(create_backend(&repo, None).is_ok());
    }

    #[test]
    fn should_look_up_capabilities_for_every_kind() {
        for repo in [
            ForgeRepository::github("github.com", "a", "b"),
            ForgeRepository::gitlab("gitlab.com", "a", "b"),
            ForgeRepository::azure_devops("dev.azure.com", "a", "b"),
            ForgeRepository::gitea("gitea.example.com", "a", "b"),
            ForgeRepository::forgejo("codeberg.org", "a", "b"),
        ] {
            let caps = capabilities(&repo, None).expect("every kind has a default profile");
            assert_eq!(caps.kind, repo.kind);
        }
    }
}
