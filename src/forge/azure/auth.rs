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
/// 1. `az account get-access-token --resource <ado-resource-id>` — Bearer
///    auth via the Azure CLI's own Entra/AAD session (whatever the caller
///    is already logged into via `az login`). Tried first per Microsoft's
///    Entra OAuth guide, which positions CLI/Entra tokens as the primary
///    supported auth path and PATs as the fallback for
///    non-interactive/service scenarios.
/// 2. `$AZURE_DEVOPS_EXT_PAT` — Basic auth, empty username + PAT password.
///
/// Returns a typed, non-panicking error — never a guessed/empty token —
/// when neither source is available.
pub fn resolve_auth() -> Result<AzureAuth> {
    if let Some(token) = fetch_az_cli_access_token()? {
        return Ok(AzureAuth {
            header_value: format!("Bearer {token}"),
            source: "az account get-access-token",
        });
    }

    if let Ok(pat) = std::env::var(PAT_ENV_VAR) {
        let trimmed = pat.trim();
        if !trimmed.is_empty() {
            return Ok(AzureAuth {
                header_value: basic_auth_header(trimmed),
                source: "AZURE_DEVOPS_EXT_PAT",
            });
        }
    }

    Err(TuicrError::UnsupportedOperation(format!(
        "no Azure DevOps credential found: run `az login` so `az account get-access-token \
         --resource {AZURE_DEVOPS_RESOURCE_ID}` can mint one, or set ${PAT_ENV_VAR} to a \
         personal access token"
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
    use crate::forge::azure::test_support::env_mutation_lock;

    /// Runs `body` with `$AZURE_DEVOPS_EXT_PAT` set to `value` **and**
    /// `$PATH` cleared so `fetch_az_cli_access_token`'s `Command::new("az")`
    /// always resolves to "not found", never a real `az` binary.
    ///
    /// This is load-bearing, not incidental: this crate's tests must never
    /// perform a live network call (this task's constraint is offline/
    /// mock-only; no live Azure DevOps or Entra ID sandbox is approved).
    /// Without clearing `$PATH`, a host that already has `az login` state
    /// (observed on at least one shared development host used for this
    /// ticket) would make `resolve_auth()`'s new Entra-first precedence
    /// silently shell out to the real `az account get-access-token`
    /// command and mint a genuine bearer token during a supposedly-offline
    /// unit test — exactly the live contact this ticket forbids.
    ///
    /// Uses `test_support::env_mutation_lock()`, a lock shared across
    /// *this* module, `contract_tests.rs`, and any other test that mutates
    /// `$AZURE_DEVOPS_EXT_PAT`/`$PATH` — a module-local mutex is not
    /// sufficient, since Rust's default parallel test harness can run this
    /// module's tests concurrently with `contract_tests`' on separate
    /// threads sharing the same process environment.
    fn with_pat_env<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = env_mutation_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let previous_pat = std::env::var(PAT_ENV_VAR).ok();
        let previous_path = std::env::var("PATH").ok();
        // SAFETY: serialized by `env_mutation_lock` above.
        unsafe {
            match value {
                Some(v) => std::env::set_var(PAT_ENV_VAR, v),
                None => std::env::remove_var(PAT_ENV_VAR),
            }
            // An empty PATH means `Command::new("az")` can never resolve to
            // any binary, real or otherwise, on any platform.
            std::env::set_var("PATH", "");
        }
        let result = body();
        unsafe {
            match &previous_pat {
                Some(v) => std::env::set_var(PAT_ENV_VAR, v),
                None => std::env::remove_var(PAT_ENV_VAR),
            }
            match &previous_path {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
        result
    }

    #[test]
    fn should_build_basic_auth_header_from_pat() {
        assert_eq!(basic_auth_header("secret"), "Basic OnNlY3JldA==");
    }

    #[test]
    fn should_resolve_pat_env_var_as_basic_auth_when_az_cli_unavailable() {
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
            // Basic header. With `$PATH` cleared (see `with_pat_env`), the
            // `az` CLI path deterministically reports "not available" and
            // this falls through to the typed error, never a guessed
            // empty-PAT header.
            let result = resolve_auth();
            match result {
                Err(TuicrError::UnsupportedOperation(msg)) => {
                    assert!(msg.contains("AZURE_DEVOPS_EXT_PAT"));
                }
                other => panic!("expected typed UnsupportedOperation, got {other:?}"),
            }
        });
    }

    #[test]
    fn should_prefer_az_cli_entra_token_over_pat_when_both_available() {
        // Mirrors the audit's mandated precedence (Entra/`az` token first,
        // `AZURE_DEVOPS_EXT_PAT` second) without ever invoking a real `az`
        // binary: a fake `az` shim script on `$PATH` stands in for the
        // real CLI, proving `resolve_auth` tries it before falling back to
        // the PAT env var, entirely offline.
        let _guard = env_mutation_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let previous_pat = std::env::var(PAT_ENV_VAR).ok();
        let previous_path = std::env::var("PATH").ok();

        let dir = std::env::temp_dir().join(format!("tuicr-az-shim-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create shim dir");
        let shim_path = dir.join("az");
        std::fs::write(&shim_path, "#!/bin/sh\necho fake-entra-token\n").expect("write shim");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&shim_path)
                .expect("stat shim")
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&shim_path, perms).expect("chmod shim");
        }

        // SAFETY: serialized by `env_mutation_lock` above.
        unsafe {
            std::env::set_var(PAT_ENV_VAR, "should-not-be-used-pat");
            std::env::set_var("PATH", &dir);
        }
        let result = resolve_auth();
        unsafe {
            match &previous_pat {
                Some(v) => std::env::set_var(PAT_ENV_VAR, v),
                None => std::env::remove_var(PAT_ENV_VAR),
            }
            match &previous_path {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);

        let auth = result.expect("shim should resolve");
        assert_eq!(auth.source, "az account get-access-token");
        assert_eq!(auth.header_value, "Bearer fake-entra-token");
    }
}
