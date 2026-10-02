//! Binding and startup configuration.
//!
//! Kept separate from `main` so the rules that matter — loopback only, no port zero,
//! a real port — are testable without opening a socket.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// The port the server binds by default.
pub const DEFAULT_PORT: u16 = 7317;

/// The loopback address the server is permitted to bind.
///
/// ADR-0001 makes the tool's central guarantee that only an explicit Approve reaches
/// Slack. Binding any other interface would expose the local API — and the Approve
/// endpoint — to the network, so this is checked rather than assumed.
pub const BIND_ADDR: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// Configuration for one server run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
        }
    }
}

impl ServerConfig {
    /// Build a config from the environment, falling back to the default port.
    pub fn from_env() -> Self {
        let port = std::env::var("SLACKBOT_PORT")
            .ok()
            .and_then(|raw| raw.parse::<u16>().ok())
            .filter(|port| *port > 0)
            .unwrap_or(DEFAULT_PORT);

        Self { port }
    }

    /// The address to bind.
    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(BIND_ADDR, self.port)
    }

    /// The URL to open in the browser.
    ///
    /// Always `127.0.0.1`, never `localhost`: the host resolves to `::1` on many
    /// systems, and binding only IPv4 would make the printed URL unreachable.
    pub fn url(&self) -> String {
        format!("http://{}/", self.bind_addr())
    }
}

/// Reject any address that is not loopback.
///
/// A guard rather than a comment, because the failure it prevents is silent: the tool
/// would appear to work and quietly be reachable from the local network.
pub fn ensure_loopback(addr: SocketAddr) -> Result<(), String> {
    if addr.ip().is_loopback() {
        Ok(())
    } else {
        Err(format!(
            "refusing to bind {addr}: this tool serves a local API with no \
             authentication, so it may only listen on loopback"
        ))
    }
}

/// Where the database and settings live. Under the OS data directory rather than the
/// working directory, so `cargo run` works from anywhere.
pub fn data_dir() -> std::path::PathBuf {
    if let Some(explicit) = std::env::var_os("SLACKBOT_DATA_DIR") {
        let dir = std::path::PathBuf::from(explicit);
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }

    #[cfg(target_os = "macos")]
    let dir = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Library/Application Support/slackbot");

    #[cfg(target_os = "windows")]
    let dir = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("slackbot");

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let dir = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".local/share")
        })
        .join("slackbot");

    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn db_path() -> std::path::PathBuf {
    data_dir().join("slackbot.db")
}

pub fn settings_path() -> std::path::PathBuf {
    data_dir().join("settings.toml")
}

/// The Slack `U…` id for this install, captured at setup by `auth.test`.
///
/// Not a secret, so a plain file. Without it, history filtering matches nothing and
/// every Fire looks like a quiet day.
pub fn read_slack_identity(path: &std::path::Path) -> Option<String> {
    line_value(&std::fs::read_to_string(path).ok()?, "slack_user_id")
}

pub fn read_github_login(path: &std::path::Path) -> Option<String> {
    line_value(&std::fs::read_to_string(path).ok()?, "github_login")
}

pub fn write_slack_identity(path: &std::path::Path, user_id: &str) -> std::io::Result<()> {
    append_setting(path, &format!("slack_user_id = \"{user_id}\""))
}

pub fn write_github_login(path: &std::path::Path, login: &str) -> std::io::Result<()> {
    append_setting(path, &format!("github_login = \"{login}\""))
}

/// Add or replace one `key = "value"` line, leaving the others alone.
///
/// A hand-rolled updater rather than a TOML round trip, because appending an identity
/// must not risk rewriting the endpoint and model next to it.
fn append_setting(path: &std::path::Path, line: &str) -> std::io::Result<()> {
    use std::io::Write;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let key = line.split('=').next().unwrap_or("").trim().to_string();
    let mut kept: Vec<String> = std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with(&key))
        .map(str::to_string)
        .collect();

    kept.push(line.to_string());
    let mut file = std::fs::File::create(path)?;
    for l in kept {
        writeln!(file, "{l}")?;
    }
    Ok(())
}

/// Read the LLM endpoint and model. Not secret, so a plain TOML file.
pub fn read_llm_settings(path: &std::path::Path) -> Option<crate::routes::LlmSettings> {
    let raw = std::fs::read_to_string(path).ok()?;
    let base_url = line_value(&raw, "base_url")?;
    let model = line_value(&raw, "model")?;
    Some(crate::routes::LlmSettings { base_url, model })
}

pub fn write_llm_settings(
    path: &std::path::Path,
    settings: &crate::routes::LlmSettings,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        format!(
            "# Not secret: the API key lives in the OS keychain, not here.\n\
             base_url = \"{}\"\nmodel = \"{}\"\n",
            settings.base_url.replace('"', "\\\""),
            settings.model.replace('"', "\\\"")
        ),
    )
}

/// Pull one `key = "value"` out of a two-line file. Deliberately not a TOML parser: the
/// file is written by this function and read by this function.
fn line_value(raw: &str, key: &str) -> Option<String> {
    raw.lines()
        .find(|line| line.starts_with(key))
        .and_then(|line| line.split_once('='))
        .map(|(_, value)| value.trim().trim_matches('"').to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_binds_loopback_on_default_port() {
        let config = ServerConfig::default();
        assert_eq!(config.bind_addr().port(), DEFAULT_PORT);
        assert!(config.bind_addr().ip().is_loopback());
    }

    #[test]
    fn url_uses_ipv4_loopback() {
        assert_eq!(
            ServerConfig::default().url(),
            format!("http://127.0.0.1:{DEFAULT_PORT}/")
        );
    }

    #[test]
    fn llm_settings_roundtrip_through_a_plain_file() {
        let dir = std::env::temp_dir().join(format!("slackbot-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("llm.toml");

        let settings = crate::routes::LlmSettings {
            base_url: "http://100.92.234.10:8000/v1".into(),
            model: "Coot AI".into(),
        };
        write_llm_settings(&path, &settings).unwrap();
        assert_eq!(read_llm_settings(&path).unwrap(), settings);

        // Only the endpoint and model are written. A comment mentions the keychain by
        // name, so check for a value that would actually leak rather than the word.
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("api_key"));
        assert!(!raw.contains("sk-"));
        assert_eq!(raw.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).count(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_settings_read_as_none() {
        assert!(read_llm_settings(std::path::Path::new("/nonexistent/llm.toml")).is_none());
    }

    #[test]
    fn non_loopback_bind_is_refused() {
        let external = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 50)), 7317);
        assert!(ensure_loopback(external).is_err());
    }

    #[test]
    fn loopback_bind_is_allowed() {
        assert!(ensure_loopback(SocketAddr::new(BIND_ADDR, DEFAULT_PORT)).is_ok());
    }

    #[test]
    fn ipv6_loopback_passes_the_guard_but_is_not_the_bind_address() {
        // ensure_loopback deliberately accepts any loopback address; BIND_ADDR is what
        // pins the choice to IPv4. Guarding both facts stops the default being widened
        // to ::1 by accident.
        let v6 = SocketAddr::new(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), DEFAULT_PORT);
        assert!(ensure_loopback(v6).is_ok());
        assert_ne!(v6.ip(), BIND_ADDR);
    }
}