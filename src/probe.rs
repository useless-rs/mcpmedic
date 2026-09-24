//! MCP server reachability probe: briefly spawn stdio processes and TCP
//! connect to remote endpoints, with strict timeouts. Zero new dependencies.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

use crate::model::Transport;

const HANDSHAKE_TIMEOUT_MS: u64 = 3000;
const EXIT_GRACE_MS: u64 = 500;
const TCP_TIMEOUT_SECS: u64 = 3;

/// Result of a reachability probe on one server.
#[derive(Debug)]
pub(crate) enum Probe {
    /// The server completed a full MCP initialize handshake; carries the
    /// server's reported name and negotiated protocol version.
    McpOk { server: String, protocol: String },
    /// The server started or the endpoint accepted a connection, but did
    /// not complete an MCP handshake.
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
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Probe::Unreachable(format!("failed to start: {e}")),
    };

    let stdin = child.stdin.take();
    if let Some(w) = stdin.as_ref() {
        let mut w = w;
        let _ = w.write_all(initialize_message().as_bytes());
        let _ = w.write_all(b"\n");
        let _ = w.flush();
    }

    let stdout = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let _ = tx.send(scan_for_jsonrpc(stdout));
    });

    let verdict = match rx.recv_timeout(Duration::from_millis(HANDSHAKE_TIMEOUT_MS)) {
        Ok(Some(line)) => classify_response(&line),
        Ok(None) => match try_wait_bounded(&mut child, EXIT_GRACE_MS) {
            Ok(Some(status)) => Probe::Unreachable(format!(
                "process exited with {status} before answering MCP initialize"
            )),
            _ => Probe::Reachable("closed stdout without an MCP response".into()),
        },
        Err(_) => match child.try_wait() {
            Ok(Some(status)) => Probe::Unreachable(format!(
                "process exited with {status} before answering MCP initialize"
            )),
            _ => Probe::Reachable(
                "no MCP initialize response within 3s (slow startup is common)".into(),
            ),
        },
    };
    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
    verdict
}

fn try_wait_bounded(
    child: &mut std::process::Child,
    max_ms: u64,
) -> std::io::Result<Option<std::process::ExitStatus>> {
    let deadline = std::time::Instant::now() + Duration::from_millis(max_ms);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(Some(status)),
            Ok(None) => {}
            Err(e) => return Err(e),
        }
        if std::time::Instant::now() >= deadline {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn initialize_message() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {
                "name": "mcpmedic",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
    .to_string()
}

fn is_jsonrpc_message(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    v.get("jsonrpc").is_some() && (v.get("result").is_some() || v.get("error").is_some())
}

fn scan_for_jsonrpc(stdout: Option<std::process::ChildStdout>) -> Option<String> {
    let out = stdout?;
    for line in std::io::BufReader::new(out).lines() {
        let Ok(line) = line else { return None };
        if is_jsonrpc_message(&line) {
            return Some(line);
        }
    }
    None
}

fn classify_response(line: &str) -> Probe {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Probe::Reachable("replied with non-JSON output".into());
    };
    if let Some(name) = v
        .pointer("/result/serverInfo/name")
        .and_then(serde_json::Value::as_str)
    {
        let protocol = v
            .pointer("/result/protocolVersion")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        return Probe::McpOk {
            server: name.to_string(),
            protocol: protocol.to_string(),
        };
    }
    if let Some(err) = v
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
    {
        return Probe::Reachable(format!("MCP error: {err}"));
    }
    Probe::Reachable("replied, but without an MCP initialize result".into())
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
    fn stdio_probe_reports_silent_runner() {
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport) {
            Probe::Reachable(msg) => {
                assert!(msg.contains("no MCP initialize response"), "{msg}");
            }
            other => panic!("expected Reachable, got {other:?}"),
        }
    }

    #[test]
    fn stdio_handshake_verifies_mcp_response() {
        let resp = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"mock","version":"1.0"}}}"#;
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec!["-c".into(), format!("read line; printf '%s\\n' '{resp}'")],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport) {
            Probe::McpOk { server, protocol } => {
                assert_eq!(server, "mock");
                assert_eq!(protocol, "2025-11-25");
            }
            other => panic!("expected McpOk, got {other:?}"),
        }
    }

    #[test]
    fn stdio_handshake_skips_banner_lines() {
        let resp = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"banner","version":"1.0"}}}"#;
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!("read line; echo 'starting up...'; printf '%s\\n' '{resp}'"),
            ],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport) {
            Probe::McpOk { server, .. } => assert_eq!(server, "banner"),
            other => panic!("expected McpOk, got {other:?}"),
        }
    }

    #[test]
    fn stdio_handshake_reports_jsonrpc_error() {
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                r#"read line; printf '%s
' '{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"boom"}}'"#
                    .into(),
            ],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport) {
            Probe::Reachable(msg) => assert!(msg.contains("boom"), "{msg}"),
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
