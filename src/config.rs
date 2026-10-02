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