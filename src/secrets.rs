//! Secrets storage.
//!
//! Behind a trait so tests substitute an in-memory map and never touch the user's login
//! keychain. Nothing here logs, formats, or returns a secret in an error message: a
//! `Display` impl on a secret type is a leak waiting to happen, so there isn't one.

#[cfg(test)]
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretKind {
    LlmApiKey,
    SlackToken,
    SlackCookie,
    GithubToken,
}

impl SecretKind {
    /// The keyring account name. Stable, so a renamed key never orphans a stored secret.
    pub fn account(&self) -> &'static str {
        match self {
            SecretKind::LlmApiKey => "llm-api-key",
            SecretKind::SlackToken => "slack-token",
            SecretKind::SlackCookie => "slack-cookie",
            SecretKind::GithubToken => "github-token",
        }
    }

    #[cfg(test)]
    pub const ALL: [SecretKind; 4] = [
        SecretKind::LlmApiKey,
        SecretKind::SlackToken,
        SecretKind::SlackCookie,
        SecretKind::GithubToken,
    ];
}

/// A stored secret. Deliberately has no `Debug` that prints its contents.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value, for handing to an HTTP client. Named to make each use visible.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether the value looks usable, without ever printing it.
    pub fn looks_present(&self) -> bool {
        !self.0.trim().is_empty()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Length only: enough to tell "wrong length" from "empty", useless to an attacker.
        f.debug_tuple("Secret")
            .field(&format_args!("<{} chars>", self.0.len()))
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("keyring unavailable: {0}")]
    Keyring(String),
    #[error("no secret stored for {kind:?}")]
    NotFound { kind: SecretKind },
}

pub trait SecretStore: Send + Sync {
    fn set(&self, kind: SecretKind, secret: &Secret) -> Result<(), SecretError>;
    fn get(&self, kind: SecretKind) -> Result<Secret, SecretError>;
    /// Remove a stored secret. Wired when a credential is cleared in Setup.
    #[allow(dead_code)]
    fn delete(&self, kind: SecretKind) -> Result<(), SecretError>;
}

/// The real store: the OS keychain, via `keyring`.
pub struct KeychainStore {
    service: String,
}

impl Default for KeychainStore {
    fn default() -> Self {
        Self {
            service: "slackbot".into(),
        }
    }
}

impl KeychainStore {
    fn entry(&self, kind: SecretKind) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(&self.service, kind.account())
            .map_err(|e| SecretError::Keyring(e.to_string()))
    }
}

impl SecretStore for KeychainStore {
    fn set(&self, kind: SecretKind, secret: &Secret) -> Result<(), SecretError> {
        self.entry(kind)?
            .set_password(secret.expose())
            .map_err(|e| SecretError::Keyring(e.to_string()))
    }

    fn get(&self, kind: SecretKind) -> Result<Secret, SecretError> {
        match self.entry(kind)?.get_password() {
            Ok(value) => Ok(Secret::new(value)),
            Err(keyring::Error::NoEntry) => Err(SecretError::NotFound { kind }),
            Err(e) => Err(SecretError::Keyring(e.to_string())),
        }
    }

    fn delete(&self, kind: SecretKind) -> Result<(), SecretError> {
        match self.entry(kind)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError::Keyring(e.to_string())),
        }
    }
}

/// An in-memory store for tests. Never used in production.
#[cfg(test)]
#[derive(Default)]
pub struct MemoryStore {
    inner: std::sync::Mutex<HashMap<SecretKind, Secret>>,
}

#[cfg(test)]
impl SecretStore for MemoryStore {
    fn set(&self, kind: SecretKind, secret: &Secret) -> Result<(), SecretError> {
        self.inner
            .lock()
            .expect("memory store mutex poisoned")
            .insert(kind, secret.clone());
        Ok(())
    }

    fn get(&self, kind: SecretKind) -> Result<Secret, SecretError> {
        self.inner
            .lock()
            .expect("memory store mutex poisoned")
            .get(&kind)
            .cloned()
            .ok_or(SecretError::NotFound { kind })
    }

    fn delete(&self, kind: SecretKind) -> Result<(), SecretError> {
        self.inner
            .lock()
            .expect("memory store mutex poisoned")
            .remove(&kind);
        Ok(())
    }
}

/// Which credentials are configured. The config file records only this.
///
/// Never the values: PRODUCT.md and ADR-0002 both require company content and tokens to
/// stay out of files on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct PresentCredentials {
    pub llm: bool,
    pub slack_token: bool,
    pub slack_cookie: bool,
    pub github: bool,
}

impl PresentCredentials {
    /// Read presence from a store without ever exposing the values.
    pub fn probe(store: &dyn SecretStore) -> Self {
        let has = |kind: SecretKind| store.get(kind).map(|s| s.looks_present()).unwrap_or(false);
        Self {
            llm: has(SecretKind::LlmApiKey),
            slack_token: has(SecretKind::SlackToken),
            slack_cookie: has(SecretKind::SlackCookie),
            github: has(SecretKind::GithubToken),
        }
    }

    /// Slack needs both halves of one session; neither alone is usable.
    pub fn slack_ready(&self) -> bool {
        self.slack_token && self.slack_cookie
    }

    /// Everything needed for a Fire to collect anything at all.
    pub fn collection_ready(&self) -> bool {
        self.slack_ready() && self.github
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrips() {
        let store = MemoryStore::default();
        store
            .set(SecretKind::LlmApiKey, &Secret::new("sk-test"))
            .unwrap();
        assert_eq!(
            store.get(SecretKind::LlmApiKey).unwrap().expose(),
            "sk-test"
        );
    }

    #[test]
    fn missing_secret_is_an_error_not_an_empty_string() {
        let store = MemoryStore::default();
        // An empty secret would authenticate as nobody and fail later, confusingly.
        assert!(matches!(
            store.get(SecretKind::GithubToken),
            Err(SecretError::NotFound { .. })
        ));
    }

    #[test]
    fn debug_never_prints_the_value() {
        let rendered = format!("{:?}", Secret::new("xoxc-super-secret-value"));
        assert!(!rendered.contains("super-secret"));
        assert!(rendered.contains("chars"));
    }

    #[test]
    fn presence_requires_both_slack_halves() {
        let only_token = PresentCredentials {
            slack_token: true,
            ..PresentCredentials::default()
        };
        assert!(!only_token.slack_ready(), "half a session is not a session");

        let both = PresentCredentials {
            slack_token: true,
            slack_cookie: true,
            ..PresentCredentials::default()
        };
        assert!(both.slack_ready());
    }

    #[test]
    fn presence_probes_without_exposing() {
        let store = MemoryStore::default();
        store
            .set(SecretKind::SlackToken, &Secret::new("xoxc-abc"))
            .unwrap();

        let present = PresentCredentials::probe(&store);
        assert!(present.slack_token);
        assert!(!present.slack_cookie);
        assert!(!present.collection_ready());
    }

    #[test]
    fn blank_values_count_as_absent() {
        let store = MemoryStore::default();
        store
            .set(SecretKind::LlmApiKey, &Secret::new("   "))
            .unwrap();
        assert!(!PresentCredentials::probe(&store).llm);
    }

    #[test]
    fn account_names_are_unique_and_stable() {
        let mut names: Vec<&str> = SecretKind::ALL.iter().map(|k| k.account()).collect();
        let count = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), count, "two kinds share an account name");
    }
}
