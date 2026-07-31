//! Minimal HTTP transport for the shared Gitea/Forgejo API surface.
//!
//! Uses `ureq` directly (already a dependency; see `src/update/check.rs` and
//! `src/update/install.rs` for the existing usage pattern) rather than
//! shelling out to a CLI, since neither Gitea nor Forgejo ships a
//! ubiquitous, review-API-capable CLI equivalent to `gh`/`glab` that this
//! codebase can assume is installed. Auth is a bearer-style `token <sha1>`
//! header (`Authorization: token ...`), the format both APIs document and
//! the one `fixtures/providers/lib/lifecycle.sh`'s `token_auth` helper uses
//! against the live disposable harnesses.
//!
//! `http_status_as_error(false)` is set deliberately: this module always
//! wants the response body alongside the status code (error bodies carry
//! the provider's own message), so status handling happens explicitly at
//! each call site instead of via `ureq::Error::StatusCode`.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use ureq::Agent;

use crate::error::{Result, TuicrError};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// A raw HTTP response: status code plus the full body text. Callers decide
/// per-endpoint which statuses are success, typed-unsupported (404/405), or
/// a hard failure.
#[derive(Debug, Clone)]
pub struct GfResponse {
    pub status: u16,
    pub body: String,
}

impl GfResponse {
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// A small HTTP client bound to one Gitea/Forgejo instance + token.
///
/// Deliberately not `#[derive(Debug)]`: a derived impl would print `token`
/// verbatim if this struct (or anything containing it) ever ends up in a
/// `{:?}`/panic message. The hand-written impl below redacts it.
pub struct GfHttpClient {
    agent: Agent,
    base_url: String,
    token: String,
}

impl std::fmt::Debug for GfHttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GfHttpClient")
            .field("base_url", &self.base_url)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl GfHttpClient {
    pub fn new(base_url: String, token: String) -> Self {
        let config = Agent::config_builder()
            .timeout_global(Some(REQUEST_TIMEOUT))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.into(),
            base_url,
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    fn auth_header(&self) -> String {
        format!("token {}", self.token)
    }

    /// Map a transport-level (connection/DNS/timeout) failure into a
    /// `TuicrError`. Never includes `self.token` in the message — only the
    /// path and the underlying `ureq` error text (which never contains
    /// request headers).
    fn transport_error(&self, path: &str, error: ureq::Error) -> TuicrError {
        TuicrError::Forge(format!(
            "request to {} failed: {error}",
            self.url(path).split('?').next().unwrap_or(path)
        ))
    }

    pub fn get(&self, path: &str) -> Result<GfResponse> {
        let response = self
            .agent
            .get(self.url(path))
            .header("Authorization", self.auth_header())
            .call()
            .map_err(|err| self.transport_error(path, err))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_string()
            .map_err(|err| TuicrError::Forge(format!("failed to read response body: {err}")))?;
        Ok(GfResponse { status, body })
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

    pub fn post_json_raw<B: Serialize>(&self, path: &str, body: &B) -> Result<GfResponse> {
        let response = self
            .agent
            .post(self.url(path))
            .header("Authorization", self.auth_header())
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|err| self.transport_error(path, err))?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|err| TuicrError::Forge(format!("failed to read response body: {err}")))?;
        Ok(GfResponse { status, body: text })
    }

    pub fn delete(&self, path: &str) -> Result<GfResponse> {
        let response = self
            .agent
            .delete(self.url(path))
            .header("Authorization", self.auth_header())
            .call()
            .map_err(|err| self.transport_error(path, err))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_string()
            .map_err(|err| TuicrError::Forge(format!("failed to read response body: {err}")))?;
        Ok(GfResponse { status, body })
    }
}

/// Turn a non-2xx response into a `TuicrError::Forge`, including the
/// endpoint path, status, and (truncated) response body for debugging.
/// Never includes the `Authorization` header value.
pub fn require_success(response: &GfResponse, path: &str) -> Result<()> {
    if response.is_success() {
        return Ok(());
    }
    let body_preview: String = response.body.chars().take(500).collect();
    Err(TuicrError::Forge(format!(
        "{path} returned HTTP {}: {body_preview}",
        response.status
    )))
}
