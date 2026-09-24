//! MCP server reachability probe: briefly spawn stdio processes and TCP
//! connect to remote endpoints, with strict timeouts. Zero new dependencies.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::model::Transport;
use crate::probe_stdio::probe_stdio;

/// Per-exchange probe time budget. Derived from one base (the
/// handshake wait) so every phase scales together: discover gets a
/// third, tools two thirds, TCP connect the same in seconds.
#[derive(Clone, Copy)]
pub(crate) struct ProbeBudget {
    pub discover_ms: u64,
    pub handshake_ms: u64,
    pub tools_ms: u64,
    pub tcp_secs: u64,
}

impl ProbeBudget {
    pub(crate) fn from_base(timeout_ms: u64) -> Self {
        Self {
            discover_ms: timeout_ms / 3,
            handshake_ms: timeout_ms,
            tools_ms: timeout_ms.saturating_mul(2) / 3,
            tcp_secs: (timeout_ms / 1000).max(1),
        }
    }
}

impl Default for ProbeBudget {
    fn default() -> Self {
        Self::from_base(3000)
    }
}

/// Result of a reachability probe on one server.
#[derive(Debug)]
pub(crate) enum Probe {
    /// The server completed a full MCP initialize handshake; carries the
    /// server's reported name, negotiated protocol version, and — when
    /// the tools/list round trip also succeeded — exposed tool count.
    McpOk {
        server: String,
        protocol: String,
        tools: Option<usize>,
    },
    /// A modern (2026-07-28-era) server answered `server/discover`;
    /// carries its reported name, supported protocol versions, and —
    /// when the modern tools/list round trip also succeeded — exposed
    /// tool count.
    ModernOk {
        server: String,
        versions: Vec<String>,
        tools: Option<usize>,
    },
    /// The server started or the endpoint accepted a connection, but did
    /// not complete an MCP handshake.
    Reachable(String),
    /// The process crashed on startup or the TCP connect failed.
    Unreachable(String),
    /// The URL could not be parsed or the probe was skipped.
    Skipped(String),
}

/// Probe one server's transport for reachability.
pub(crate) fn probe_transport(transport: &Transport, budget: ProbeBudget) -> Probe {
    match transport {
        Transport::Stdio { command, args, env } => probe_stdio(command, args, env, budget),
        Transport::Remote { url, .. } => probe_remote(url, budget),
    }
}

fn probe_remote(url: &str, budget: ProbeBudget) -> Probe {
    let Some((host, port)) = extract_host_port(url) else {
        return Probe::Skipped(format!("cannot parse URL `{url}`"));
    };
    let addr_str = format!("{host}:{port}");
    let Ok(mut addrs) = addr_str.to_socket_addrs() else {
        return Probe::Unreachable("DNS resolution failed".into());
    };
    let Some(sock_addr) = addrs.next() else {
        return Probe::Unreachable("no addresses found".into());
    };
    match TcpStream::connect_timeout(&sock_addr, Duration::from_secs(budget.tcp_secs)) {
        Ok(_) => Probe::Reachable(format!("connected to {sock_addr}")),
        Err(e) => Probe::Unreachable(format!("connect failed: {e}")),
    }
}

fn extract_host_port(url: &str) -> Option<(String, u16)> {
    let after_scheme = url.split("://").nth(1)?;
    let host_port = after_scheme.split('/').next()?;
    // Strip `user:pass@` credentials: without this, `rsplit_once(':')`
    // below splits inside the userinfo and DNS resolution misreports.
    let host_port = host_port.rsplit('@').next().unwrap_or(host_port);
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(port) => (h.to_string(), port),
            Err(_) => (
                host_port.to_string(),
                if url.starts_with("https") { 443 } else { 80 },
            ),
        },
        None => (
            host_port.to_string(),
            if url.starts_with("https") { 443 } else { 80 },
        ),
    };
    if host.is_empty() {
        return None;
    }
    Some((host, port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_parsing_handles_ports_userinfo_and_defaults() {
        assert_eq!(
            extract_host_port("https://example.com/mcp"),
            Some(("example.com".to_owned(), 443))
        );
        assert_eq!(
            extract_host_port("http://example.com:8080/mcp"),
            Some(("example.com".to_owned(), 8080))
        );
        assert_eq!(
            extract_host_port("https://user:pass@example.com/mcp"),
            Some(("example.com".to_owned(), 443))
        );
        assert!(extract_host_port("not-a-url").is_none());
    }

    #[test]
    fn remote_probe_parses_host_and_port() {
        assert_eq!(
            extract_host_port("https://api.example.com/mcp"),
            Some(("api.example.com".into(), 443))
        );
        assert_eq!(
            extract_host_port("http://localhost:3000/mcp"),
            Some(("localhost".into(), 3000))
        );
        assert_eq!(
            extract_host_port("https://example.com:8443/path"),
            Some(("example.com".into(), 8443))
        );
        assert_eq!(extract_host_port("not-a-url"), None);
    }

    #[test]
    fn remote_probe_connects_to_localhost() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/mcp");
        match probe_remote(&url, ProbeBudget::default()) {
            Probe::Reachable(msg) => assert!(msg.contains("connected"), "{msg}"),
            other => panic!("expected Reachable, got {other:?}"),
        }
    }
}
