//! Read-only GitHub account verification records. No OAuth tokens are stored here.
use crate::{Error, Result, config};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum GithubOperation {
    Read,
}

impl GithubOperation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum GithubIdentityStatus {
    Active,
    Revoked,
}

impl GithubIdentityStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }
}

/// Evidence collected by the trusted CLI after a read-only `gh api user` check.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GithubIdentityAttach {
    pub profile_id: String,
    pub host: String,
    pub login: String,
    pub account_id: u64,
    pub display_name: Option<String>,
    pub persona_id: String,
    pub expected_persona_revision: u64,
    pub operations: Vec<GithubOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GithubIdentityRevoke {
    pub identity_id: String,
    pub expected_revision: u64,
}

#[derive(Debug, Clone)]
pub struct VerifiedAccount {
    pub login: String,
    pub account_id: u64,
    pub display_name: Option<String>,
}

pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host == host.to_ascii_lowercase()
        && !host.starts_with('.')
        && !host.ends_with('.')
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        })
}

pub fn valid_login(login: &str) -> bool {
    (1..=39).contains(&login.len())
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-')
}

pub fn valid_profile_id(profile_id: &str) -> bool {
    uuid::Uuid::parse_str(profile_id).is_ok_and(|id| id.to_string() == profile_id)
}

fn command(profile_dir: &Path, host: &str, gh: &Path) -> Result<Command> {
    if !valid_host(host) {
        return Err(Error::Invalid(
            "GitHub host must be a lowercase DNS hostname without a scheme or path".into(),
        ));
    }
    config::check_private_dir(profile_dir)?;
    let mut command = Command::new(gh);
    command
        .args(["api", "--hostname", host, "user"])
        .env("GH_CONFIG_DIR", profile_dir)
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_COLOR", "1")
        .env_remove("GH_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .env_remove("GH_ENTERPRISE_TOKEN")
        .env_remove("GITHUB_ENTERPRISE_TOKEN")
        .env_remove("GH_HOST")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    Ok(command)
}

fn parse_account(bytes: &[u8]) -> Result<VerifiedAccount> {
    if bytes.len() > 65_536 {
        return Err(Error::Invalid(
            "GitHub CLI response exceeds the verification limit".into(),
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| Error::Invalid("GitHub CLI did not return a valid account object".into()))?;
    if value.get("type").and_then(serde_json::Value::as_str) != Some("User") {
        return Err(Error::Invalid(
            "GitHub CLI did not verify a user account".into(),
        ));
    }
    let login = value
        .get("login")
        .and_then(serde_json::Value::as_str)
        .filter(|login| valid_login(login))
        .ok_or_else(|| Error::Invalid("GitHub response has no valid login".into()))?
        .to_owned();
    let account_id = value
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .filter(|id| *id > 0 && *id <= i64::MAX as u64)
        .ok_or_else(|| Error::Invalid("GitHub response has no valid numeric account ID".into()))?;
    let display_name = value
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.trim().is_empty() && name.len() <= 256 && !name.contains('\0'))
        .map(str::to_owned);
    Ok(VerifiedAccount {
        login,
        account_id,
        display_name,
    })
}

/// Verify the account behind an isolated GitHub CLI profile using a read-only API call.
///
/// Token environment variables are removed so the isolated profile is authoritative.
/// Stderr is never returned because it can contain local authentication details.
pub fn verify(profile_dir: &Path, host: &str, gh: &Path) -> Result<VerifiedAccount> {
    let mut command = command(profile_dir, host, gh)?;
    let mut child = command
        .spawn()
        .map_err(|_| Error::Unavailable("GitHub CLI could not be started; install gh".into()))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Unavailable(
                "GitHub identity verification timed out".into(),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    };
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| Error::Unavailable("GitHub CLI output unavailable".into()))?
        .take(65_537)
        .read_to_end(&mut bytes)?;
    if !status.success() {
        return Err(Error::Unavailable(
            "GitHub identity verification failed; complete gh auth login for this profile".into(),
        ));
    }
    parse_account(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn private_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[test]
    fn verification_command_uses_the_isolated_profile_and_removes_tokens() {
        let dir = private_dir();
        let command = command(dir.path(), "github.com", Path::new("/usr/bin/gh")).unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, ["api", "--hostname", "github.com", "user"]);
        let envs: std::collections::HashMap<_, _> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert_eq!(
            envs.get("GH_CONFIG_DIR").unwrap().as_deref(),
            Some(dir.path().to_str().unwrap())
        );
        for token in [
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GH_ENTERPRISE_TOKEN",
            "GITHUB_ENTERPRISE_TOKEN",
            "GH_HOST",
        ] {
            assert!(envs.contains_key(token));
            assert!(envs[token].is_none());
        }
    }

    #[test]
    fn account_parsing_rejects_malformed_and_non_user_responses() {
        let account =
            parse_account(br#"{"login":"alice","id":4242,"name":"Alice Example","type":"User"}"#)
                .unwrap();
        assert_eq!(account.login, "alice");
        assert_eq!(account.account_id, 4242);
        assert_eq!(account.display_name.as_deref(), Some("Alice Example"));
        for raw in [
            &br#"{"login":"alice"}"#[..],
            &br#"{"login":"alice","id":0,"type":"User"}"#[..],
            &br#"{"login":"example","id":7,"type":"Organization"}"#[..],
            &br#"not json"#[..],
        ] {
            assert!(parse_account(raw).is_err());
        }
    }

    #[test]
    fn host_and_login_validation_rejects_ambiguous_values() {
        for host in [
            "",
            "GitHub.com",
            "-github.com",
            "github.com/path",
            "host..example",
        ] {
            assert!(!valid_host(host));
        }
        assert!(valid_host("github.example.com"));
        for login in ["", "-alice", "alice-", "alice/bob", &"a".repeat(40)] {
            assert!(!valid_login(login));
        }
        assert!(valid_login("alice-01"));
    }
}
