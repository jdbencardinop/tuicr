//! Minimal HTTP transport for the Azure DevOps REST API.
//!
//! Uses `ureq` directly (already a dependency; see
//! `crate::forge::giteafj::client` for the established pattern this
//! mirrors) rather than shelling out to a CLI: unlike Gitea/Forgejo's `tea`,
//! `az devops` is a heavier, separately-installed CLI extension this
//! codebase cannot assume is present, and it is not a general PR-review API
//! surface (no thread/vote endpoints) even when it is.
//!
//! Every request pins `api-version=7.1` (the exact version this module's
//! endpoint shapes were evidenced against — see `src/forge/azure/models.rs`
//! and `src/forge/azure/fixtures/README.md`) so a future Azure DevOps
//! server-side default-version bump can never silently change the wire
//! shape this client expects.
//!
//! `http_status_as_error(false)` is set deliberately, exactly as in
//! `giteafj::client`: this module always wants the response body alongside
//! the status code (error bodies carry Azure DevOps' own `message` field),
//! so status handling happens explicitly at each call site.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::Agent;

use crate::error::{Result, TuicrError};
use crate::forge::azure::auth::AzureAuth;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Response header Azure DevOps sets on a paginated response that has more
/// pages available (e.g. `GET .../pullrequests/{id}/commits`). Present only
/// when more data exists; its value becomes the next request's
/// `continuationToken` query parameter. See
/// <https://learn.microsoft.com/en-us/rest/api/azure/devops/git/pull-request-commits/get-pull-request-commits>.
pub const CONTINUATION_TOKEN_HEADER: &str = "x-ms-continuationtoken";

/// A raw HTTP response: status code, full body text, and the response
/// headers a caller may need beyond the body (currently only the
/// continuation-token header). Callers decide per-endpoint which statuses
/// are success, typed-unsupported (404/405), or a hard failure.
#[derive(Debug, Clone)]
pub struct AdoResponse {
    pub status: u16,
    pub body: String,
    pub continuation_token: Option<String>,
}

impl AdoResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A small HTTP client bound to one Azure DevOps organization/collection
/// base URL + resolved credential.
///
/// Deliberately not `#[derive(Debug)]`: a derived impl would print
/// `auth.header_value` verbatim if this struct ever ends up in a `{:?}`/
/// panic message. `AzureAuth` already redacts itself (see
/// `crate::forge::azure::auth::AzureAuth`'s hand-written `Debug`), so the
/// derive here would actually be safe today, but writing it out explicitly
/// keeps that guarantee independent of `AzureAuth`'s own impl ever
/// changing.
pub struct AdoHttpClient {
    agent: Agent,
    base_url: String,
    auth: AzureAuth,
}

impl std::fmt::Debug for AdoHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdoHttpClient")
            .field("base_url", &self.base_url)
            .field("auth", &self.auth)
            .finish()
    }
}

impl AdoHttpClient {
    pub fn new(base_url: String, auth: AzureAuth) -> Self {
        let config = Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
            base_url,
            auth,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    /// Map a transport-level (connection/DNS/timeout) failure into a
    /// `TuicrError`. Never includes `self.auth`'s header value in the
    /// message — only the path (with any query string stripped) and the
    /// underlying `ureq` error text (which never contains request
    /// headers).
    fn transport_error(&self, path: &str, error: ureq::Error) -> TuicrError {
        TuicrError::Forge(format!(
            "request to {} failed: {error}",
            self.url(path).split('?').next().unwrap_or(path)
        ))
    }

    fn read_response(
        &self,
        response: ureq::http::Response<ureq::Body>,
        path: &str,
    ) -> Result<AdoResponse> {
        let status = response.status().as_u16();
        let continuation_token = response
            .headers()
            .get(CONTINUATION_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        let body = response.into_body().read_to_string().map_err(|err| {
            TuicrError::Forge(format!(
                "failed to read response body from {}: {err}",
                self.url(path).split('?').next().unwrap_or(path)
            ))
        })?;
        Ok(AdoResponse {
            status,
            body,
            continuation_token,
        })
    }

    pub fn get(&self, path: &str) -> Result<AdoResponse> {
        let response = self
            .agent
            .get(self.url(path))
            .header("Authorization", &self.auth.header_value)
            .header("Accept", "application/json")
            .call()
            .map_err(|err| self.transport_error(path, err))?;
        self.read_response(response, path)
    }

    pub fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self.get(path)?;
        require_success(&response, path)?;
        serde_json::from_str(&response.body).map_err(TuicrError::from)
    }

    pub fn post_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let response = self.post_json_raw(path, body)?;
        require_success(&response, path)?;
        serde_json::from_str(&response.body).map_err(TuicrError::from)
    }

    pub fn post_json_raw<B: Serialize>(&self, path: &str, body: &B) -> Result<AdoResponse> {
        let response = self
            .agent
            .post(self.url(path))
            .header("Authorization", &self.auth.header_value)
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|err| self.transport_error(path, err))?;
        self.read_response(response, path)
    }

    pub fn patch_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let response = self
            .agent
            .patch(self.url(path))
            .header("Authorization", &self.auth.header_value)
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|err| self.transport_error(path, err))?;
        let response = self.read_response(response, path)?;
        require_success(&response, path)?;
        serde_json::from_str(&response.body).map_err(TuicrError::from)
    }

    pub fn put_json<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T> {
        let response = self
            .agent
            .put(self.url(path))
            .header("Authorization", &self.auth.header_value)
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|err| self.transport_error(path, err))?;
        let response = self.read_response(response, path)?;
        require_success(&response, path)?;
        serde_json::from_str(&response.body).map_err(TuicrError::from)
    }
}

/// Turn a non-2xx response into a `TuicrError::Forge`, including the
/// endpoint path, status, and (truncated) response body for debugging.
/// Never includes the `Authorization` header value.
pub fn require_success(response: &AdoResponse, path: &str) -> Result<()> {
    if response.is_success() {
        return Ok(());
    }
    let body_preview: String = response.body.chars().take(500).collect();
    Err(TuicrError::Forge(format!(
        "{path} returned HTTP {}: {body_preview}",
        response.status
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_report_success_for_2xx_status() {
        let response = AdoResponse {
            status: 201,
            body: String::new(),
            continuation_token: None,
        };
        assert!(response.is_success());
    }

    #[test]
    fn should_report_failure_for_non_2xx_status() {
        let response = AdoResponse {
            status: 404,
            body: "not found".to_string(),
            continuation_token: None,
        };
        assert!(!response.is_success());
        assert!(require_success(&response, "/some/path").is_err());
    }

    #[test]
    fn should_never_display_auth_header_via_debug() {
        let auth = AzureAuth {
            header_value: "Basic super-secret-value".to_string(),
            source: "AZURE_DEVOPS_EXT_PAT",
        };
        let client = AdoHttpClient::new("https://dev.azure.com".to_string(), auth);
        let debug_output = format!("{client:?}");
        assert!(!debug_output.contains("super-secret-value"));
        assert!(debug_output.contains("<redacted>"));
    }
}
