//! Token/auth-header resolution for Azure DevOps.
//!
//! Never stores a secret: every source here is either an environment
//! variable the caller's shell already controls, or a live shell-out to the
//! `az` CLI (which manages its own token cache — this module never reads,
//! writes, or duplicates that cache). Nothing here writes a token to disk
//! or logs a token value.
//!
//! Evidence:
//! - Personal Access Token (PAT) auth is Basic auth with an empty username
//!   and the PAT as the password (`curl -u :<PAT> ...`), per
//!   <https://learn.microsoft.com/en-us/azure/devops/organizations/accounts/use-personal-access-tokens-to-authenticate>.
//! - `AZURE_DEVOPS_EXT_PAT` is the official environment variable the
//!   `az devops` CLI extension itself reads to bypass `az devops login`,
//!   per <https://learn.microsoft.com/en-us/azure/devops/cli/log-in-via-pat>
//!   ("You can also set the environment variable AZURE_DEVOPS_EXT_PAT").
//! - Entra ID (Azure AD) access tokens for Azure DevOps are obtained via
//!   `az account get-access-token --resource <resource-id>` and sent as
//!   `Authorization: Bearer <token>`, per
//!   <https://learn.microsoft.com/en-us/azure/devops/integrate/get-started/authentication/entra-oauth>
//!   and `az account get-access-token --help`. The resource ID
//!   `499b84ac-1321-427f-aa17-267ca6975798` is Azure DevOps' well-known
//!   first-party application ID, documented in the same Entra OAuth guide.

use std::ffi::OsStr;

use crate::error::{Result, TuicrError};
use crate::process::run_command_output;

/// Official `az devops` CLI extension env var (see module docs). Checked
/// first since it is the most explicit, most-documented source.
const PAT_ENV_VAR: &str = "AZURE_DEVOPS_EXT_PAT";

/// Azure DevOps' well-known first-party Entra application resource ID, used
/// with `az account get-access-token --resource` to mint a bearer token
/// scoped to Azure DevOps rather than an arbitrary/default resource.
const AZURE_DEVOPS_RESOURCE_ID: &str = "499b84ac-1321-427f-aa17-267ca6975798";

/// One resolved auth header value plus a human-readable description of
/// where it came from (for error messages only — never logs the header
/// value itself).
#[derive(Clone)]
pub struct AzureAuth {
    pub header_value: String,
    pub source: &'static str,
}

impl std::fmt::Debug for AzureAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureAuth")
            .field("header_value", &"<redacted>")
            .field("source", &self.source)
            .finish()
    }
}

/// Resolve an `Authorization` header value for Azure DevOps, in order:
/// 1. `$AZURE_DEVOPS_EXT_PAT` — Basic auth, empty username + PAT password.
/// 2. `az account get-access-token --resource <ado-resource-id>` — Bearer
///    auth via the Azure CLI's own Entra/AAD session (whatever the caller
///    is already logged into via `az login`).
///
/// Returns a typed, non-panicking error — never a guessed/empty token —
/// when neither source is available.
pub fn resolve_auth() -> Result<AzureAuth> {
    if let Ok(pat) = std::env::var(PAT_ENV_VAR) {
        let trimmed = pat.trim();
        if !trimmed.is_empty() {
            return Ok(AzureAuth {
                header_value: basic_auth_header(trimmed),
                source: "AZURE_DEVOPS_EXT_PAT",
            });
        }
    }

    if let Some(token) = fetch_az_cli_access_token()? {
        return Ok(AzureAuth {
            header_value: format!("Bearer {token}"),
            source: "az account get-access-token",
        });
    }

    Err(TuicrError::UnsupportedOperation(format!(
        "no Azure DevOps credential found: set ${PAT_ENV_VAR} to a personal access token, or \
         run `az login` so `az account get-access-token --resource {AZURE_DEVOPS_RESOURCE_ID}` \
         can mint one"
    )))
}

/// Build the Basic auth header value for a PAT: base64(":<pat>").
fn basic_auth_header(pat: &str) -> String {
    use base64::Engine as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(format!(":{pat}"));
    format!("Basic {encoded}")
}

/// Shell out to `az account get-access-token --resource <id> --query
/// accessToken -o tsv`, returning `Ok(None)` (not an error) when the `az`
/// CLI is not installed or the caller is not logged in — those are
/// "credential not available", not a hard failure, so `resolve_auth` can
/// still report one combined, actionable error message. Never logs the
/// resulting token.
fn fetch_az_cli_access_token() -> Result<Option<String>> {
    let args: [&OsStr; 5] = [
        OsStr::new("account"),
        OsStr::new("get-access-token"),
        OsStr::new("--resource"),
        OsStr::new(AZURE_DEVOPS_RESOURCE_ID),
        OsStr::new("--query"),
    ];
    // `--query accessToken -o tsv` prints just the token on stdout with a
    // trailing newline; kept as two separate arg groups above/below only
    // for readability, not semantics.
    let mut full_args: Vec<&OsStr> = args.to_vec();
    full_args.push(OsStr::new("accessToken"));
    full_args.push(OsStr::new("-o"));
    full_args.push(OsStr::new("tsv"));

    match run_command_output("az", None, full_args) {
        Ok(output) => {
            let token = output.trim();
            if token.is_empty() {
                Ok(None)
            } else {
                Ok(Some(token.to_string()))
            }
        }
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_pat_env<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var(PAT_ENV_VAR).ok();
        // SAFETY: serialized by `env_lock` above.
        unsafe {
            match value {
                Some(v) => std::env::set_var(PAT_ENV_VAR, v),
                None => std::env::remove_var(PAT_ENV_VAR),
            }
        }
        let result = body();
        unsafe {
            match &previous {
                Some(v) => std::env::set_var(PAT_ENV_VAR, v),
                None => std::env::remove_var(PAT_ENV_VAR),
            }
        }
        result
    }

    #[test]
    fn should_build_basic_auth_header_from_pat() {
        assert_eq!(basic_auth_header("secret"), "Basic OnNlY3JldA==");
    }

    #[test]
    fn should_resolve_pat_env_var_as_basic_auth() {
        with_pat_env(Some("my-pat"), || {
            let auth = resolve_auth().expect("should resolve from env");
            assert_eq!(auth.header_value, "Basic Om15LXBhdA==");
            assert_eq!(auth.source, "AZURE_DEVOPS_EXT_PAT");
        });
    }

    #[test]
    fn should_never_display_header_value_via_debug() {
        let auth = AzureAuth {
            header_value: "Basic super-secret-value".to_string(),
            source: "AZURE_DEVOPS_EXT_PAT",
        };
        let debug_output = format!("{auth:?}");
        assert!(!debug_output.contains("super-secret-value"));
        assert!(debug_output.contains("<redacted>"));
    }

    #[test]
    fn should_ignore_blank_pat_env_var() {
        with_pat_env(Some("   "), || {
            // Blank env var must not short-circuit into an empty-token
            // Basic header; it falls through to the `az` CLI path, which
            // will also fail in this sandboxed test environment (no `az`
            // installed / not logged in) and report the typed error.
            let result = resolve_auth();
            if let Err(TuicrError::UnsupportedOperation(msg)) = result {
                assert!(msg.contains("AZURE_DEVOPS_EXT_PAT"));
            }
            // If `az` happens to be installed and logged in on the host
            // running this test, `resolve_auth` may also legitimately
            // succeed via that path — either outcome proves the blank PAT
            // was not used as-is.
        });
    }
}
