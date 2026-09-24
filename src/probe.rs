//! MCP server reachability probe: briefly spawn stdio processes and TCP
//! connect to remote endpoints, with strict timeouts. Zero new dependencies.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use std::time::Duration;

use crate::model::Transport;

const EXIT_GRACE_MS: u64 = 500;
const STDERR_TAIL_CHARS: usize = 300;
const STDERR_HANDOFF_MS: u64 = 150;

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

fn probe_stdio(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    budget: ProbeBudget,
) -> Probe {
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

    let mut stdin = child.stdin.take();
    if let Some(w) = stdin.as_mut() {
        let _ = w.write_all(discover_message().as_bytes());
        let _ = w.write_all(b"\n");
        let _ = w.flush();
    }

    let stdout = child.stdout.take();
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        if let Some(out) = stdout {
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if is_jsonrpc_message(&line) && tx.send(Some(line)).is_err() {
                    break;
                }
            }
        }
        let _ = tx.send(None);
    });

    let stderr = child.stderr.take();
    let (err_tx, err_rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut tail = String::new();
        if let Some(err) = stderr {
            for line in std::io::BufReader::new(err).lines() {
                let Ok(line) = line else { break };
                let trimmed = line.trim();
                if trimmed.is_empty() || tail.len() >= STDERR_TAIL_CHARS {
                    continue;
                }
                if !tail.is_empty() {
                    tail.push_str(" | ");
                }
                tail.push_str(trimmed);
            }
        }
        let _ = err_tx.send(tail);
    });

    let verdict = match rx.recv_timeout(Duration::from_millis(budget.discover_ms)) {
        Ok(Some(line)) => match classify_discover(&line) {
            DiscoverOutcome::Modern { server, versions } => {
                let tools = fetch_modern_tool_count(&mut stdin, &rx, budget);
                Probe::ModernOk {
                    server,
                    versions,
                    tools,
                }
            }
            DiscoverOutcome::Legacy => {
                send_legacy_initialize(&mut stdin);
                legacy_handshake_phase(&mut stdin, &rx, &mut child, &err_rx, budget)
            }
        },
        Ok(None) => match try_wait_bounded(&mut child, EXIT_GRACE_MS) {
            Ok(Some(status)) => Probe::Unreachable(exit_message(status, &err_rx)),
            _ => Probe::Reachable("closed stdout without an MCP response".into()),
        },
        Err(_) => {
            send_legacy_initialize(&mut stdin);
            legacy_handshake_phase(&mut stdin, &rx, &mut child, &err_rx, budget)
        }
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

enum DiscoverOutcome {
    Modern {
        server: String,
        versions: Vec<String>,
    },
    Legacy,
}

fn classify_discover(line: &str) -> DiscoverOutcome {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return DiscoverOutcome::Legacy;
    };
    if let Some(versions) = v
        .pointer("/result/supportedVersions")
        .and_then(serde_json::Value::as_array)
    {
        let versions: Vec<String> = versions
            .iter()
            .filter_map(|x| x.as_str().map(str::to_string))
            .collect();
        let server = v
            .pointer("/result/_meta/io.modelcontextprotocol~1serverInfo/name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        return DiscoverOutcome::Modern { server, versions };
    }
    if v.pointer("/error/code").and_then(serde_json::Value::as_i64) == Some(-32022) {
        let versions: Vec<String> = v
            .pointer("/error/data/supported")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        return DiscoverOutcome::Modern {
            server: "unknown".into(),
            versions,
        };
    }
    DiscoverOutcome::Legacy
}

fn fetch_modern_tool_count(
    stdin: &mut Option<std::process::ChildStdin>,
    rx: &std::sync::mpsc::Receiver<Option<String>>,
    budget: ProbeBudget,
) -> Option<usize> {
    let w = stdin.as_mut()?;
    let _ = w.write_all(modern_tools_list_message().as_bytes());
    let _ = w.write_all(b"\n");
    let _ = w.flush();
    let deadline = std::time::Instant::now() + Duration::from_millis(budget.tools_ms);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match rx.recv_timeout(remaining) {
            Ok(Some(line)) => match tools_count_from_response(&line) {
                ToolsReply::Count(count) => return Some(count),
                ToolsReply::Unknown => return None,
                ToolsReply::NotIt => {}
            },
            _ => return None,
        }
    }
}

fn modern_tools_list_message() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "mcpmedic",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    })
    .to_string()
}

fn discover_message() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/discover",
        "params": {
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {
                    "name": "mcpmedic",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "io.modelcontextprotocol/clientCapabilities": {}
            }
        }
    })
    .to_string()
}

fn send_legacy_initialize(stdin: &mut Option<std::process::ChildStdin>) {
    if let Some(w) = stdin.as_mut() {
        let _ = w.write_all(initialize_message().as_bytes());
        let _ = w.write_all(b"\n");
        let _ = w.flush();
    }
}

fn legacy_handshake_phase(
    stdin: &mut Option<std::process::ChildStdin>,
    rx: &std::sync::mpsc::Receiver<Option<String>>,
    child: &mut std::process::Child,
    err_rx: &std::sync::mpsc::Receiver<String>,
    budget: ProbeBudget,
) -> Probe {
    match rx.recv_timeout(Duration::from_millis(budget.handshake_ms)) {
        Ok(Some(line)) => match parse_success(&line) {
            Some((server, protocol)) => {
                let tools = fetch_tool_count(stdin, rx, budget);
                Probe::McpOk {
                    server,
                    protocol,
                    tools,
                }
            }
            None => classify_failure(&line),
        },
        Ok(None) => match try_wait_bounded(child, EXIT_GRACE_MS) {
            Ok(Some(status)) => Probe::Unreachable(exit_message(status, err_rx)),
            _ => Probe::Reachable("closed stdout without an MCP response".into()),
        },
        Err(_) => match child.try_wait() {
            Ok(Some(status)) => Probe::Unreachable(exit_message(status, err_rx)),
            _ => Probe::Reachable(format!(
                "no MCP initialize response within {}s (slow startup is common)",
                (budget.handshake_ms / 1000).max(1)
            )),
        },
    }
}

fn exit_message(
    status: std::process::ExitStatus,
    err_rx: &std::sync::mpsc::Receiver<String>,
) -> String {
    let mut msg = format!("process exited with {status} before answering the MCP probe");
    let tail = err_rx
        .recv_timeout(Duration::from_millis(STDERR_HANDOFF_MS))
        .unwrap_or_default();
    if !tail.is_empty() {
        msg.push_str(" — stderr: ");
        msg.push_str(&tail);
    }
    msg
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

fn fetch_tool_count(
    stdin: &mut Option<std::process::ChildStdin>,
    rx: &std::sync::mpsc::Receiver<Option<String>>,
    budget: ProbeBudget,
) -> Option<usize> {
    let w = stdin.as_mut()?;
    let _ = w.write_all(initialized_notification().as_bytes());
    let _ = w.write_all(b"\n");
    let _ = w.write_all(tools_list_message().as_bytes());
    let _ = w.write_all(b"\n");
    let _ = w.flush();
    let deadline = std::time::Instant::now() + Duration::from_millis(budget.tools_ms);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match rx.recv_timeout(remaining) {
            Ok(Some(line)) => match tools_count_from_response(&line) {
                ToolsReply::Count(count) => return Some(count),
                ToolsReply::Unknown => return None,
                ToolsReply::NotIt => {}
            },
            _ => return None,
        }
    }
}

fn initialized_notification() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })
    .to_string()
}

fn tools_list_message() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    })
    .to_string()
}

#[derive(Debug, PartialEq, Eq)]
enum ToolsReply {
    NotIt,
    Count(usize),
    Unknown,
}

fn tools_count_from_response(line: &str) -> ToolsReply {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return ToolsReply::NotIt;
    };
    if v.get("id").and_then(serde_json::Value::as_i64) != Some(2) {
        return ToolsReply::NotIt;
    }
    match v
        .pointer("/result/tools")
        .and_then(serde_json::Value::as_array)
    {
        Some(tools) => ToolsReply::Count(tools.len()),
        None => ToolsReply::Unknown,
    }
}

fn parse_success(line: &str) -> Option<(String, String)> {
    let v = serde_json::from_str::<serde_json::Value>(line).ok()?;
    let name = v
        .pointer("/result/serverInfo/name")
        .and_then(serde_json::Value::as_str)?;
    let protocol = v
        .pointer("/result/protocolVersion")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    Some((name.to_string(), protocol.to_string()))
}

fn classify_failure(line: &str) -> Probe {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return Probe::Reachable("replied with non-JSON output".into());
    };
    if let Some(err) = v
        .pointer("/error/message")
        .and_then(serde_json::Value::as_str)
    {
        return Probe::Reachable(format!("MCP error: {err}"));
    }
    Probe::Reachable("replied, but without an MCP initialize result".into())
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
        match probe_transport(&transport, ProbeBudget::default()) {
            Probe::Reachable(msg) => {
                assert!(msg.contains("no MCP initialize response"), "{msg}");
            }
            other => panic!("expected Reachable, got {other:?}"),
        }
    }

    #[test]
    fn stdio_handshake_verifies_mcp_response() {
        let legacy_err =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#;
        let init = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"mock","version":"1.0"}}}"#;
        let tools = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"a"},{"name":"b"}]}}"#;
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "read a; printf '%s\\n' '{legacy_err}'; read b; printf '%s\\n' '{init}'; read c; read d; printf '%s\\n' '{tools}'"
                ),
            ],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport, ProbeBudget::default()) {
            Probe::McpOk {
                server,
                protocol,
                tools,
            } => {
                assert_eq!(server, "mock");
                assert_eq!(protocol, "2025-11-25");
                assert_eq!(tools, Some(2));
            }
            other => panic!("expected McpOk with tools, got {other:?}"),
        }
    }

    #[test]
    fn stdio_handshake_skips_banner_lines() {
        let legacy_err =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#;
        let init = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"banner","version":"1.0"}}}"#;
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "read a; printf '%s\\n' '{legacy_err}'; echo 'starting up...'; read b; printf '%s\\n' '{init}'; read c; read d"
                ),
            ],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport, ProbeBudget::default()) {
            Probe::McpOk { server, tools, .. } => {
                assert_eq!(server, "banner");
                assert_eq!(tools, None, "server exits after initialize: tools unknown");
            }
            other => panic!("expected McpOk, got {other:?}"),
        }
    }

    #[test]
    fn tools_count_matches_only_id_2_responses() {
        assert_eq!(
            tools_count_from_response(
                r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"a"}]}}"#
            ),
            ToolsReply::Count(1)
        );
        assert_eq!(
            tools_count_from_response(r#"{"jsonrpc":"2.0","id":3,"result":{"tools":[]}}"#),
            ToolsReply::NotIt
        );
        assert_eq!(
            tools_count_from_response(r#"{"jsonrpc":"2.0","id":2,"error":{"message":"nope"}}"#),
            ToolsReply::Unknown
        );
        assert_eq!(tools_count_from_response("not json"), ToolsReply::NotIt);
    }

    #[test]
    fn stdio_handshake_reports_jsonrpc_error() {
        let legacy_err =
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#;
        let boom = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"boom"}}"#;
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!("read a; printf '%s\\n' '{legacy_err}'; read b; printf '%s\\n' '{boom}'"),
            ],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport, ProbeBudget::default()) {
            Probe::Reachable(msg) => assert!(msg.contains("boom"), "{msg}"),
            other => panic!("expected Reachable, got {other:?}"),
        }
    }

    #[test]
    fn stdio_discover_reports_modern_server() {
        let discover_result = r#"{"jsonrpc":"2.0","id":1,"result":{"resultType":"complete","supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"ExampleServer","version":"1.0.0"}}}}"#;
        let tools = r#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","tools":[{"name":"a"},{"name":"b"},{"name":"c"}]}}"#;
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                format!(
                    "read a; printf '%s\\n' '{discover_result}'; read b; printf '%s\\n' '{tools}'"
                ),
            ],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport, ProbeBudget::default()) {
            Probe::ModernOk {
                server,
                versions,
                tools,
            } => {
                assert_eq!(server, "ExampleServer");
                assert_eq!(versions, vec!["2026-07-28".to_string()]);
                assert_eq!(tools, Some(3));
            }
            other => panic!("expected ModernOk, got {other:?}"),
        }
    }

    #[test]
    fn stdio_discover_unsupported_version_reports_modern() {
        let err = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32022,"message":"Unsupported protocol version","data":{"supported":["2026-07-28","2025-11-25"],"requested":"1900-01-01"}}}"#;
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec!["-c".into(), format!("read a; printf '%s\\n' '{err}'")],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport, ProbeBudget::default()) {
            Probe::ModernOk {
                server,
                versions,
                tools,
            } => {
                assert_eq!(server, "unknown");
                assert_eq!(versions.len(), 2);
                assert!(versions.contains(&"2026-07-28".to_string()));
                assert_eq!(tools, None);
            }
            other => panic!("expected ModernOk, got {other:?}"),
        }
    }

    #[test]
    fn stdio_probe_includes_stderr_tail() {
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec![
                "-c".into(),
                "echo 'fatal: missing module' >&2; exit 3".into(),
            ],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport, ProbeBudget::default()) {
            Probe::Unreachable(msg) => {
                assert!(msg.contains("exited"), "{msg}");
                assert!(msg.contains("fatal: missing module"), "{msg}");
            }
            other => panic!("expected Unreachable, got {other:?}"),
        }
    }

    #[test]
    fn stdio_probe_catches_immediate_exit() {
        let transport = Transport::Stdio {
            command: "sh".into(),
            args: vec!["-c".into(), "exit 1".into()],
            env: BTreeMap::new(),
        };
        match probe_transport(&transport, ProbeBudget::default()) {
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
        match probe_transport(&transport, ProbeBudget::default()) {
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
        match probe_remote(&url, ProbeBudget::default()) {
            Probe::Reachable(msg) => assert!(msg.contains("connected"), "{msg}"),
            other => panic!("expected Reachable, got {other:?}"),
        }
    }
}
