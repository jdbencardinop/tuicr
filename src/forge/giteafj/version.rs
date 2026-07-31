//! Live `/api/v1/version` detection and Gitea/Forgejo cross-validation.
//!
//! Forgejo's version string carries a `+gitea-<base>` build-metadata suffix
//! (e.g. `"16.0.1+gitea-1.22.0"`); Gitea's does not (e.g. `"1.24.7"`) — see
//! `fixtures/providers/results-examples/{forgejo-16.0.1,gitea-1.24.7}.example.json`,
//! key `server_version`, and `fixtures/providers/lib/lifecycle.sh`'s
//! `jget "$HTTP_BODY" "version"` call against the real `/api/v1/version`
//! response. This module uses that fingerprint to catch operator
//! misconfiguration (e.g. `--provider forgejo` pointed at a plain Gitea
//! host) as a typed error instead of silently applying the wrong
//! capability profile.

use crate::error::{Result, TuicrError};
use crate::forge::capabilities::ProviderVersion;
use crate::forge::giteafj::client::GfHttpClient;
use crate::forge::giteafj::models::GfVersionResponse;
use crate::forge::traits::ForgeKind;

/// Build-metadata marker Forgejo appends to its own version string. Gitea's
/// version string never contains this.
const FORGEJO_FINGERPRINT: &str = "+gitea-";

/// Fetch `/api/v1/version` and parse the reported version string.
pub fn fetch_version(client: &GfHttpClient) -> Result<ProviderVersion> {
    let response: GfVersionResponse = client.get_json("/api/v1/version")?;
    Ok(ProviderVersion::new(response.version))
}

/// Cross-check that a live version string is consistent with the
/// configured `kind`, using the Forgejo build-metadata fingerprint.
/// Returns a typed `UnsupportedOperation` error (never a panic, never a
/// silent capability substitution) on mismatch.
pub fn verify_kind_matches(kind: ForgeKind, version: &ProviderVersion) -> Result<()> {
    let is_forgejo_shaped = version.0.contains(FORGEJO_FINGERPRINT);
    match kind {
        ForgeKind::Forgejo if !is_forgejo_shaped => Err(TuicrError::UnsupportedOperation(format!(
            "configured provider is `forgejo`, but the live server's version string \
             (`{}`) does not carry Forgejo's `{FORGEJO_FINGERPRINT}` build-metadata marker; \
             this looks like a plain Gitea instance — check the repository's configured \
             provider kind",
            version.0
        ))),
        ForgeKind::Gitea if is_forgejo_shaped => Err(TuicrError::UnsupportedOperation(format!(
            "configured provider is `gitea`, but the live server's version string \
             (`{}`) carries Forgejo's `{FORGEJO_FINGERPRINT}` build-metadata marker; \
             this looks like a Forgejo instance — check the repository's configured \
             provider kind",
            version.0
        ))),
        ForgeKind::Gitea | ForgeKind::Forgejo => Ok(()),
        ForgeKind::GitHub | ForgeKind::GitLab | ForgeKind::AzureDevOps => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_accept_gitea_kind_for_plain_version_string() {
        let version = ProviderVersion::new("1.24.7");
        assert!(verify_kind_matches(ForgeKind::Gitea, &version).is_ok());
    }

    #[test]
    fn should_accept_forgejo_kind_for_fingerprinted_version_string() {
        let version = ProviderVersion::new("16.0.1+gitea-1.22.0");
        assert!(verify_kind_matches(ForgeKind::Forgejo, &version).is_ok());
    }

    #[test]
    fn should_reject_forgejo_kind_for_plain_gitea_version_string() {
        let version = ProviderVersion::new("1.24.7");
        let err = verify_kind_matches(ForgeKind::Forgejo, &version).unwrap_err();
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }

    #[test]
    fn should_reject_gitea_kind_for_fingerprinted_forgejo_version_string() {
        let version = ProviderVersion::new("16.0.1+gitea-1.22.0");
        let err = verify_kind_matches(ForgeKind::Gitea, &version).unwrap_err();
        assert!(matches!(err, TuicrError::UnsupportedOperation(_)));
    }
}
