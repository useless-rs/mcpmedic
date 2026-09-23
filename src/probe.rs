//! MCP server reachability probe: briefly spawn stdio processes and TCP
//! connect to remote endpoints, with strict timeouts. Zero new dependencies.

use std::collections::BTreeMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

use crate::model::Transport;

const STDIO_GRACE_MS: u64 = 500;
const TCP_TIMEOUT_SECS: u64 = 3;

/// Result of a reachability probe on one server.
#[derive(Debug)]
pub(crate) enum Probe {
    /// The server started or the endpoint accepted a connection.
    Reachable(String),
    /// The process crashed on startup or the TCP connect failed.
    Unreachable(String),
    /// The URL could not be parsed or the probe was skipped.
    Skipped(String),
}

/// Probe one server's transport for reachability.
pub(crate) fn probe_transport(transport: &Transport) -> Probe {
    match transport {
        Transport::Stdio { command, args, env } => probe_stdio(command, args, env),
        Transport::Remote { url, .. } => probe_remote(url),
    }
}

fn probe_stdio(command: &str, args: &[String], env: &BTreeMap<String, String>) -> Probe {
    if command.contains("${") || command.contains('%') {
        return Probe::Skipped("templated command — resolved by the tool itself".into());
    }
    let mut cmd = Command::new(command);
    cmd.args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, v) in env {
        cmd.env(k, v);
    }
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::sleep(Duration::from_millis(STDIO_GRACE_MS));
            match child.try_wait() {
                Ok(Some(status)) => {
                    Probe::Unreachable(format!("process exited immediately with {status}"))
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    Probe::Reachable("process started".into())
                }
                Err(e) => Probe::Unreachable(format!("probe error: {e}")),
            }
        }
        Err(e) => Probe::Unreachable(format!("failed to start: {e}")),
    }
}

fn probe_remote(url: &str) -> Probe {
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
    match TcpStream::connect_timeout(&sock_addr, Duration::from_secs(TCP_TIMEOUT_SECS)) {
        Ok(_) => Probe::Reachable(format!("connected to {sock_addr}")),
        Err(e) => Probe::Unreachable(format!("connect failed: {e}")),
    }
}

fn extract_host_port(url: &str) -> Option<(String, u16)> {
    let after_scheme = url.split("://").nth(1)?;
    let host_port = after_scheme.split('/').next()?;
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
    fn stdio_probe_starts_and_kills_sh() {
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec![],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport) {
            Probe::Reachable(msg) => assert!(msg.contains("started"), "{msg}"),
            other => panic!("expected Reachable, got {other:?}"),
        }
    }

    #[test]
    fn stdio_probe_catches_immediate_exit() {
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec!["-c".into(), "exit 1".into()],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport) {
            Probe::Unreachable(msg) => assert!(msg.contains("exited"), "{msg}"),
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    #[test]
    fn stdio_probe_skips_templated_commands() {
        let transport = Transport::Stdio {
            command: "${MY_TOOL}".into(),
            args: vec![],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport) {
            Probe::Skipped(_) => {}
            other => panic!("expected Skipped, got {other:?}"),
        }
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
        match probe_remote(&url) {
            Probe::Reachable(msg) => assert!(msg.contains("connected"), "{msg}"),
            other => panic!("expected Reachable, got {other:?}"),
        }
    }
}
