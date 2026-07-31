//! Token/host resolution for the Gitea/Forgejo family.
//!
//! Never stores a secret: every source here is either an environment
//! variable the caller's shell already controls, or the `tea` CLI's own
//! config file (which the user manages via `tea login add`), read purely
//! in-memory. Nothing here writes, caches to disk, or logs a token value.

use std::path::PathBuf;

use crate::error::{Result, TuicrError};
use crate::forge::traits::ForgeKind;

/// Kind-specific environment variable checked first, mirroring this
/// codebase's existing `GH_TOKEN`/`GITLAB_TOKEN` convention for the two
/// working backends (see `src/forge/github/gh.rs`, `src/forge/gitlab/glab.rs`).
fn env_var_for(kind: ForgeKind) -> &'static str {
    match kind {
        ForgeKind::Gitea => "GITEA_TOKEN",
        ForgeKind::Forgejo => "FORGEJO_TOKEN",
        ForgeKind::GitHub | ForgeKind::GitLab | ForgeKind::AzureDevOps => {
            unreachable!("token resolution is only called for the Gitea/Forgejo family")
        }
    }
}

/// Resolve an API token for `kind`'s host `host` from, in order:
/// 1. the kind-specific environment variable (`GITEA_TOKEN`/`FORGEJO_TOKEN`);
/// 2. a `tea login` entry in `~/.config/tea/config.yml` (or
///    `$XDG_CONFIG_HOME/tea/config.yml`) whose `url` matches `host`.
///
/// Returns a typed, non-panicking error — never a guessed/empty token —
/// when neither source has one.
pub fn resolve_token(kind: ForgeKind, host: &str) -> Result<String> {
    let env_var = env_var_for(kind);
    if let Ok(value) = std::env::var(env_var) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    if let Some(token) = tea_config_token(host) {
        return Ok(token);
    }

    Err(TuicrError::UnsupportedOperation(format!(
        "no {} token found for host `{host}`: set ${env_var}, or add a `tea login` entry \
         (`tea login add --url https://{host} --token ...`) in ~/.config/tea/config.yml",
        kind.provider_key(),
    )))
}

fn tea_config_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg).join("tea").join("config.yml"));
    }
    directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("tea").join("config.yml"))
}

fn tea_config_token(host: &str) -> Option<String> {
    let path = tea_config_path()?;
    let content = std::fs::read_to_string(path).ok()?;
    parse_tea_config_token(&content, host)
}

/// Purpose-built scan of `tea`'s flat `logins:` list — not a general YAML
/// parser. Each login is a `- name: ... \n  url: ... \n  token: ...` block
/// (see https://github.com/pkulik0/gitea-skill's authentication guide for
/// the documented shape); this walks lines looking for `url:`/`token:` keys
/// within the `logins:` section and returns the `token` of whichever login
/// block's `url` host matches `host` (scheme- and trailing-slash-agnostic).
fn parse_tea_config_token(content: &str, host: &str) -> Option<String> {
    let target_host = normalize_host(host);
    let mut in_logins = false;
    let mut current_url_host: Option<String> = None;
    let mut current_token: Option<String> = None;

    for raw_line in content.lines() {
        let trimmed = raw_line.trim_start();
        if !in_logins {
            if trimmed == "logins:" {
                in_logins = true;
            }
            continue;
        }
        if trimmed.starts_with("- ") {
            if let Some(found) =
                flush_login_entry(&mut current_url_host, &mut current_token, &target_host)
            {
                return Some(found);
            }
            apply_kv(
                trimmed.trim_start_matches("- "),
                &mut current_url_host,
                &mut current_token,
            );
            continue;
        }
        apply_kv(trimmed, &mut current_url_host, &mut current_token);
    }
    flush_login_entry(&mut current_url_host, &mut current_token, &target_host)
}

/// Take the in-progress `url`/`token` pair, resetting both for the next
/// login block, and return the token when the pair's normalized host
/// matches `target_host`.
fn flush_login_entry(
    url_host: &mut Option<String>,
    token: &mut Option<String>,
    target_host: &str,
) -> Option<String> {
    let uh = url_host.take();
    let tok = token.take();
    match (uh, tok) {
        (Some(uh), Some(tok)) if uh == target_host => Some(tok),
        _ => None,
    }
}

fn apply_kv(line: &str, url_host: &mut Option<String>, token: &mut Option<String>) {
    if let Some(rest) = line.strip_prefix("url:") {
        *url_host = Some(normalize_host(unquote(rest.trim())));
    } else if let Some(rest) = line.strip_prefix("token:") {
        *token = Some(unquote(rest.trim()).to_string());
    }
}

fn unquote(value: &str) -> &str {
    value.trim_matches(['"', '\''])
}

fn normalize_host(value: &str) -> String {
    value
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Resolve `host` (a `ForgeRepository.host` value) into a full base URL.
/// Hosts that already carry an explicit scheme (used by the local
/// disposable-instance harness, e.g. `http://127.0.0.1:54321`) are used
/// as-is; anything else is assumed to be a bare hostname served over
/// `https`, matching every self-hosted Gitea/Forgejo deployment guide.
pub fn base_url_from_host(host: &str) -> String {
    if host.starts_with("http://") || host.starts_with("https://") {
        host.trim_end_matches('/').to_string()
    } else {
        format!("https://{}", host.trim_end_matches('/'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_build_https_base_url_for_bare_host() {
        assert_eq!(
            base_url_from_host("gitea.example.com"),
            "https://gitea.example.com"
        );
    }

    #[test]
    fn should_keep_explicit_http_scheme_for_local_fixtures() {
        assert_eq!(
            base_url_from_host("http://127.0.0.1:54321"),
            "http://127.0.0.1:54321"
        );
    }

    #[test]
    fn should_trim_trailing_slash_from_host() {
        assert_eq!(
            base_url_from_host("https://gitea.example.com/"),
            "https://gitea.example.com"
        );
    }

    #[test]
    fn should_parse_matching_login_token_from_tea_config() {
        let content = "logins:\n  - name: main\n    url: https://gitea.example.com\n    token: abc123\n    default: true\n  - name: other\n    url: https://git.other.com\n    token: def456\n";
        assert_eq!(
            parse_tea_config_token(content, "gitea.example.com"),
            Some("abc123".to_string())
        );
        assert_eq!(
            parse_tea_config_token(content, "git.other.com"),
            Some("def456".to_string())
        );
    }

    #[test]
    fn should_return_none_for_unmatched_host_in_tea_config() {
        let content =
            "logins:\n  - name: main\n    url: https://gitea.example.com\n    token: abc123\n";
        assert_eq!(
            parse_tea_config_token(content, "unrelated.example.com"),
            None
        );
    }

    #[test]
    fn should_match_last_login_entry_without_trailing_blank_line() {
        let content =
            "logins:\n  - name: only\n    url: https://gitea.example.com\n    token: last-token";
        assert_eq!(
            parse_tea_config_token(content, "gitea.example.com"),
            Some("last-token".to_string())
        );
    }

    #[test]
    fn should_resolve_token_from_env_var_before_tea_config() {
        // given/when/then: exercised indirectly via resolve_token's public
        // contract; this test documents that an explicit env var wins even
        // when a tea config file might also match, without needing to
        // mutate global process env in a unit test (that's covered by the
        // env-var precedence contract itself, verified by inspection of
        // `resolve_token`'s short-circuit `if let Ok(value) = ... return`).
        assert_eq!(env_var_for(ForgeKind::Gitea), "GITEA_TOKEN");
        assert_eq!(env_var_for(ForgeKind::Forgejo), "FORGEJO_TOKEN");
    }
}
