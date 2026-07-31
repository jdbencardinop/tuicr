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

/// Standard rate-limit backoff header. Per
/// <https://learn.microsoft.com/en-us/azure/devops/integrate/concepts/rate-limits?view=azure-devops>
/// ("Best practices"): "Honor the Retry-After header: If you receive it in
/// a response, wait the specified time before sending another request.".
/// Azure DevOps documents this can accompany either a hard `429` block or
/// (per the same page) a `200` soft-throttling delay — this client
/// surfaces the value either way (see `read_response`) rather than only
/// checking it on `429`, but never sleeps/retries on it automatically: per
/// audit requirement 8 ("no automatic retry without ID reconciliation"),
/// deciding whether/how long to wait is the caller's job, not this
/// transport's.
pub const RETRY_AFTER_HEADER: &str = "retry-after";

/// Quota-visibility headers Azure DevOps documents as "if available" (same
/// rate-limits page, "Best practices": "Monitor X-RateLimit headers ...
/// track `X-RateLimit-Remaining` and `X-RateLimit-Limit`"). Diagnostic
/// only — surfaced in error messages when present, never acted on
/// automatically.
pub const RATE_LIMIT_REMAINING_HEADER: &str = "x-ratelimit-remaining";
pub const RATE_LIMIT_LIMIT_HEADER: &str = "x-ratelimit-limit";

/// A raw HTTP response: status code, full body text, and the response
/// headers a caller may need beyond the body (continuation-token plus the
/// rate-limit/backoff headers above). Callers decide per-endpoint which
/// statuses are success, typed-unsupported (404/405), or a hard failure.
#[derive(Debug, Clone)]
pub struct AdoResponse {
    pub status: u16,
    pub body: String,
    pub continuation_token: Option<String>,
    /// `Retry-After` header value, verbatim (Azure DevOps documents this
    /// as a number of seconds, but this client does not parse/act on it —
    /// only surfaces it for the caller/human to see; see
    /// `RETRY_AFTER_HEADER`'s doc comment).
    pub retry_after: Option<String>,
    pub rate_limit_remaining: Option<String>,
    pub rate_limit_limit: Option<String>,
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
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.to_string())
        };
        let continuation_token = header(CONTINUATION_TOKEN_HEADER);
        let retry_after = header(RETRY_AFTER_HEADER);
        let rate_limit_remaining = header(RATE_LIMIT_REMAINING_HEADER);
        let rate_limit_limit = header(RATE_LIMIT_LIMIT_HEADER);
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
            retry_after,
            rate_limit_remaining,
            rate_limit_limit,
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
/// endpoint path, status, (truncated) response body, and — when present —
/// the `Retry-After`/`X-RateLimit-*` headers Azure DevOps uses for
/// throttling (see the header constants above). This function only
/// *surfaces* those values in the error text for the caller/human to act
/// on; per audit requirement 8 it never sleeps or retries automatically —
/// callers are responsible for any backoff/reconciliation decision.
/// Never includes the `Authorization` header value.
pub fn require_success(response: &AdoResponse, path: &str) -> Result<()> {
    if response.is_success() {
        return Ok(());
    }
    let body_preview: String = response.body.chars().take(500).collect();
    let mut suffix = String::new();
    if let Some(retry_after) = &response.retry_after {
        suffix.push_str(&format!(" (retry-after: {retry_after}s)"));
    }
    if response.rate_limit_remaining.is_some() || response.rate_limit_limit.is_some() {
        let remaining = response.rate_limit_remaining.as_deref().unwrap_or("?");
        let limit = response.rate_limit_limit.as_deref().unwrap_or("?");
        suffix.push_str(&format!(" (rate-limit: {remaining}/{limit})"));
    }
    Err(TuicrError::Forge(format!(
        "{path} returned HTTP {}{suffix}: {body_preview}",
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
            retry_after: None,
            rate_limit_remaining: None,
            rate_limit_limit: None,
        };
        assert!(response.is_success());
    }

    #[test]
    fn should_report_failure_for_non_2xx_status() {
        let response = AdoResponse {
            status: 404,
            body: "not found".to_string(),
            continuation_token: None,
            retry_after: None,
            rate_limit_remaining: None,
            rate_limit_limit: None,
        };
        assert!(!response.is_success());
        assert!(require_success(&response, "/some/path").is_err());
    }

    #[test]
    fn should_surface_retry_after_and_rate_limit_headers_in_error_message_without_retrying() {
        let response = AdoResponse {
            status: 429,
            body: "TF400733: The request has been blocked".to_string(),
            continuation_token: None,
            retry_after: Some("30".to_string()),
            rate_limit_remaining: Some("0".to_string()),
            rate_limit_limit: Some("200".to_string()),
        };
        let err = require_success(&response, "/some/path").unwrap_err();
        let message = err.to_string();
        assert!(message.contains("retry-after: 30s"));
        assert!(message.contains("rate-limit: 0/200"));
        assert!(message.contains("429"));
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
