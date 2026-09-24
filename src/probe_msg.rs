//! Probe wire format: JSON-RPC message builders, reply classifiers,
//! and tool-count extraction shared by the stdio handshake phases.
//!
//! Split out of [`crate::probe_stdio`].

use crate::probe::Probe;

pub(crate) enum DiscoverOutcome {
    Modern {
        server: String,
        versions: Vec<String>,
    },
    Legacy,
}

pub(crate) fn classify_discover(line: &str) -> DiscoverOutcome {
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

pub(crate) fn modern_tools_list_message() -> String {
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

pub(crate) fn discover_message() -> String {
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

pub(crate) fn initialize_message() -> String {
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

pub(crate) fn is_jsonrpc_message(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    v.get("jsonrpc").is_some() && (v.get("result").is_some() || v.get("error").is_some())
}

pub(crate) fn initialized_notification() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })
    .to_string()
}

pub(crate) fn tools_list_message() -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    })
    .to_string()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ToolsReply {
    NotIt,
    Count(usize),
    Unknown,
}

pub(crate) fn tools_count_from_response(line: &str) -> ToolsReply {
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

pub(crate) fn parse_success(line: &str) -> Option<(String, String)> {
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

pub(crate) fn classify_failure(line: &str) -> Probe {
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
