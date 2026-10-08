use std::path::PathBuf;

use serde::Deserialize;

use crate::{AppError, AppResult};

/// Presence of this section requires verified email for human registration.
/// Secrets stay in files on the server, never in public discovery or Rinx.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistrationEmailConfig {
    pub agentmail_inbox: String,
    pub agentmail_api_key_file: PathBuf,
    /// Independent random secret (at least 32 bytes) for keyed OTP digests.
    pub otp_secret_file: PathBuf,
    /// A loopback HTTP URL is allowed for local integration tests only.
    #[serde(default = "default_api_url")]
    pub agentmail_api_url: String,
}

fn default_api_url() -> String {
    "https://api.agentmail.to/v0/".to_owned()
}

impl RegistrationEmailConfig {
    pub fn read_secret(path: &std::path::Path) -> AppResult<String> {
        let value = std::fs::read_to_string(path)
            .map_err(|_| AppError::internal("Cannot read registration email secret file"))?;
        let value = value.trim().to_owned();
        if value.len() < 32 || value.contains(['\r', '\n']) {
            return Err(AppError::internal(
                "Registration email secrets must contain at least 32 bytes on one line",
            ));
        }
        Ok(value)
    }

    pub fn validate(&self) -> AppResult<()> {
        Self::read_secret(&self.agentmail_api_key_file)?;
        Self::read_secret(&self.otp_secret_file)?;
        if self.agentmail_inbox.is_empty()
            || self.agentmail_inbox.len() > 254
            || self.agentmail_inbox.chars().any(char::is_control)
        {
            return Err(AppError::internal("Invalid AgentMail sender inbox"));
        }
        let url = url::Url::parse(&self.agentmail_api_url)
            .map_err(|_| AppError::internal("Invalid AgentMail API URL"))?;
        let local = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
        if !(url.scheme() == "https" || url.scheme() == "http" && local)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(AppError::internal(
                "AgentMail requires HTTPS (HTTP is allowed only on loopback)",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transport_and_secrets_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("key");
        std::fs::write(&key, "x".repeat(32)).unwrap();
        let mut config = RegistrationEmailConfig {
            agentmail_inbox: "sender@agentmail.to".into(),
            agentmail_api_key_file: key.clone(),
            otp_secret_file: key,
            agentmail_api_url: default_api_url(),
        };
        assert!(config.validate().is_ok());
        for invalid in [
            "http://external.example/v0/",
            "https://user:secret@example.org/v0/",
            "https://api.agentmail.to/v0/?key=secret",
        ] {
            config.agentmail_api_url = invalid.into();
            assert!(config.validate().is_err());
        }
        config.agentmail_api_url = "http://127.0.0.1:1234/v0/".into();
        assert!(config.validate().is_ok());
        std::fs::write(&config.otp_secret_file, "short").unwrap();
        assert!(config.validate().is_err());
    }
}
