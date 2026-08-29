//! Automatic checks remain disabled until the fork owns a stable release
//! channel. The inherited upstream endpoint must never replace this binary.
const FORK_UPDATE_CHECK_MESSAGE: &str = "tuicr-fork: automatic update checks are disabled in this fork build; this binary will never contact crates.io.";

#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub is_ahead: bool,
}

pub enum UpdateCheckResult {
    UpdateAvailable(UpdateInfo),
    UpToDate(UpdateInfo),
    AheadOfRelease(UpdateInfo),
    Failed(String),
}

pub fn check_for_updates() -> UpdateCheckResult {
    UpdateCheckResult::Failed(FORK_UPDATE_CHECK_MESSAGE.to_string())
}

/// Still used by the (now unreachable from this binary, but intact and
/// tested) real update-installer logic in `super::install`.
pub(super) fn is_newer_version(current: &str, latest: &str) -> bool {
    semver::Version::parse(latest)
        .ok()
        .zip(semver::Version::parse(current).ok())
        .is_some_and(|(latest_version, current_version)| latest_version > current_version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_contacts_the_network_and_reports_fork_message() {
        assert!(matches!(
            check_for_updates(),
            UpdateCheckResult::Failed(ref msg) if msg.contains("fork build") && msg.contains("never contact crates.io")
        ));
    }

    #[test]
    fn compares_supported_and_invalid_versions() {
        assert!(is_newer_version("0.5.0", "0.6.0"));
        assert!(is_newer_version("0.5.0", "1.0.0"));
        assert!(is_newer_version("0.5.0", "0.5.1"));
        assert!(is_newer_version("1.0.0-beta.1", "1.0.0"));
        assert!(is_newer_version("1.0.0-beta.1", "1.0.0-beta.2"));
        assert!(!is_newer_version("0.5", "0.5.1"));
        assert!(!is_newer_version("0.5.0", "0.5.0"));
        assert!(!is_newer_version("0.6.0", "0.5.0"));
        assert!(!is_newer_version("dev", "1.0.0"));
        assert!(!is_newer_version("1.0.0", "dev"));
        assert!(!is_newer_version("1", "2"));
    }
}
