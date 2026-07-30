//! Bounded provider registry: capability lookup plus a typed-error
//! transport factory.
//!
//! This replaces the previously closed `ForgeKind` match in
//! `crate::app::create_forge_backend`. GitHub and GitLab construct their
//! existing `gh`/`glab` transports; Azure DevOps, Gitea, and Forgejo are
//! registered identities with capability profiles
//! (`crate::forge::capabilities`) but no transport — attempting to build a
//! backend for one of them returns
//! `Err(TuicrError::UnsupportedOperation(_))`, never a panic and never a
//! silent fallback to a different kind.

use std::path::PathBuf;

use crate::error::{Result, TuicrError};
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
/// Returns `Ok` for `ForgeKind::GitHub`/`ForgeKind::GitLab`, reusing the
/// existing `gh`/`glab`-shelling backends unchanged. Returns a typed
/// `Err(TuicrError::UnsupportedOperation(_))` for the placeholder kinds
/// (`AzureDevOps`/`Gitea`/`Forgejo`) — no transport exists for them yet, and
/// this factory must never panic or silently substitute another kind's
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
        ForgeKind::AzureDevOps | ForgeKind::Gitea | ForgeKind::Forgejo => {
            Err(TuicrError::UnsupportedOperation(format!(
                "{} has no transport yet; only capability profiles and dry-run planning are \
                 supported for it (see docs/follow-on-map/tickets/12-implement-azure-adapter.md \
                 and 13-implement-gitea-forgejo-adapter.md)",
                repo.kind.provider_key()
            )))
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
    fn should_return_typed_error_for_azure_devops_placeholder() {
        let repo = ForgeRepository::azure_devops("dev.azure.com", "org", "project");
        let err = match create_backend(&repo, None) {
            Ok(_) => panic!("expected no Azure DevOps transport yet"),
            Err(err) => err,
        };
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_return_typed_error_for_gitea_placeholder() {
        let repo = ForgeRepository::gitea("gitea.example.com", "owner", "repo");
        let err = match create_backend(&repo, None) {
            Ok(_) => panic!("expected no Gitea transport yet"),
            Err(err) => err,
        };
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_return_typed_error_for_forgejo_placeholder() {
        let repo = ForgeRepository::forgejo("codeberg.org", "owner", "repo");
        let err = match create_backend(&repo, None) {
            Ok(_) => panic!("expected no Forgejo transport yet"),
            Err(err) => err,
        };
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_look_up_capabilities_for_every_kind_without_a_transport() {
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
