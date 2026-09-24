//! JSON entry read/write: normalized model <-> each tool's dialect.
//!
//! Split out of [`crate::format`]. Shared primitives (`Format`,
//! `remote_url_of`, `string_map`, `string_array`) stay in `format`.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::format::{Format, remote_url_of, string_array, string_map};
use crate::model::{Servers, Transport};
use crate::registry::{DisableFlag, ToolId};

/// Read the server map out of a parsed JSON document.
pub(crate) fn json_servers(format: Format, doc: &Value) -> (Servers, Vec<String>) {
    let Some(key) = format.servers_key() else {
        return (Servers::new(), Vec::new());
    };
    let Some(map) = doc.get(key).and_then(Value::as_object) else {
        return (Servers::new(), Vec::new());
    };
    json_entries(map)
}

/// opencode keeps servers under `mcp` — either flat (v1) or nested under
/// `mcp.servers` (v2). Support both when reading.
pub(crate) fn openclaw_servers(doc: &Value) -> (Servers, Vec<String>) {
    let Some(Value::Object(map)) = doc.get("mcp").and_then(|m| m.get("servers")) else {
        return (Servers::new(), Vec::new());
    };
    json_entries(map)
}

pub(crate) fn opencode_servers(doc: &Value) -> (Servers, Vec<String>) {
    let mut servers = Servers::new();
    let mut problems = Vec::new();
    let Some(mcp) = doc.get("mcp").and_then(Value::as_object) else {
        return (servers, problems);
    };
    for (name, entry) in mcp {
        if name == "servers" {
            if let Some(inner) = entry.as_object() {
                let (nested, mut p) = json_entries(inner);
                for (n, t) in nested {
                    servers.insert(n, t);
                }
                problems.append(&mut p);
            }
        }
    }
    let (flat, mut p) = json_entries(mcp);
    for (n, t) in flat {
        servers.insert(n, t);
    }
    problems.append(&mut p);
    (servers, problems)
}

/// Parse a `name -> entry` JSON object into the normalized model.
fn json_entries(map: &Map<String, Value>) -> (Servers, Vec<String>) {
    let mut servers = Servers::new();
    let mut problems = Vec::new();
    for (name, entry) in map {
        if name == "servers" {
            continue; // opencode v2 nesting, handled by the caller
        }
        match parse_json_entry(entry) {
            Ok(t) => {
                servers.insert(name.clone(), t);
            }
            Err(msg) => problems.push(format!("server `{name}`: {msg}")),
        }
    }
    (servers, problems)
}

/// Parse one JSON server entry into a [`Transport`].
pub(crate) fn parse_json_entry(entry: &Value) -> Result<Transport, String> {
    let Value::Object(obj) = entry else {
        return Err("entry is not a JSON object".into());
    };

    let declared = obj.get("type").and_then(Value::as_str);
    let remote_types = ["http", "streamable-http", "sse", "remote"];

    // opencode style: `command` is an array of command + args.
    if let Some(Value::Array(parts)) = obj.get("command") {
        let strings: Option<Vec<String>> = parts
            .iter()
            .map(|p| p.as_str().map(str::to_owned))
            .collect();
        if let Some(mut argv) = strings {
            if let Some(command) = if argv.is_empty() {
                None
            } else {
                Some(argv.remove(0))
            } {
                let env = string_map(obj.get("environment").or_else(|| obj.get("env")));
                return Ok(Transport::Stdio {
                    command,
                    args: argv,
                    env,
                });
            }
        }
        return Err("`command` array is empty".into());
    }

    // Remote server: a URL field wins.
    if let Some(url) = remote_url_of(obj) {
        if let Some(t) = declared {
            if t == "stdio" || t == "local" {
                return Err(format!("type `{t}` but a remote URL field is set"));
            }
            if !remote_types.contains(&t) {
                return Err(format!("unknown type `{t}` with a remote URL field set"));
            }
        }
        let headers = string_map(
            obj.get("headers")
                .or_else(|| obj.get("httpHeaders"))
                .or_else(|| obj.get("http_headers")),
        );
        return Ok(Transport::Remote { url, headers });
    }

    // Zed legacy layout: `command` is an object with a `path` key.
    if let Some(Value::Object(cmd)) = obj.get("command") {
        if let Some(Value::String(path)) = cmd.get("path") {
            let args = string_array(cmd.get("args").or_else(|| obj.get("args")));
            let env = string_map(cmd.get("env").or_else(|| obj.get("env")));
            return Ok(Transport::Stdio {
                command: path.clone(),
                args,
                env,
            });
        }
    }

    // Plain stdio server.
    if let Some(Value::String(command)) = obj.get("command") {
        if let Some(t) = declared {
            if remote_types.contains(&t) {
                return Err(format!(
                    "type `{t}` indicates a remote server but no remote URL (`url`/`httpUrl`/`serverUrl`) is set"
                ));
            }
        }
        let args = string_array(obj.get("args"));
        let env = string_map(obj.get("env"));
        return Ok(Transport::Stdio {
            command: command.clone(),
            args,
            env,
        });
    }

    if let Some(t) = declared {
        if remote_types.contains(&t) {
            return Err(format!(
                "type `{t}` indicates a remote server but no remote URL (`url`/`httpUrl`/`serverUrl`) is set"
            ));
        }
    }
    Err("entry has neither `command` nor a remote URL field (`url`/`httpUrl`/`serverUrl`)".into())
}

/// Build the JSON entry a tool expects from a normalized [`Transport`].
///
/// Remote entries are rendered in each tool's documented dialect: the Claude
/// tools key on `type: "http"`, Roo Code on the literal `streamable-http`,
/// Cline on `streamableHttp`, Gemini CLI and Qwen Code on their `httpUrl` field, Windsurf on
/// `serverUrl`, and Cursor, Kimi Code and Zed on a plain `url` with the transport inferred.
pub(crate) fn build_json_entry(format: Format, tool: ToolId, transport: &Transport) -> Value {
    let mut entry = Map::new();
    match transport {
        Transport::Stdio { command, args, env } => {
            if matches!(format, Format::Vscode | Format::Crush) {
                entry.insert("type".into(), json!("stdio"));
            }
            entry.insert("command".into(), json!(command));
            if !args.is_empty() {
                entry.insert("args".into(), json!(args));
            }
            if !env.is_empty() {
                entry.insert("env".into(), json!(env));
            }
        }
        Transport::Remote { url, headers } => {
            match (format, tool) {
                (Format::McpServers, ToolId::RooCode) => {
                    entry.insert("type".into(), json!("streamable-http"));
                    entry.insert("url".into(), json!(url));
                }
                (Format::McpServers, ToolId::Cline) => {
                    entry.insert("type".into(), json!("streamableHttp"));
                    entry.insert("url".into(), json!(url));
                }
                (Format::McpServers, ToolId::GeminiCli | ToolId::QwenCode) => {
                    entry.insert("httpUrl".into(), json!(url));
                }
                (Format::McpServers, ToolId::Windsurf | ToolId::Antigravity) => {
                    entry.insert("serverUrl".into(), json!(url));
                }
                (Format::McpServers, ToolId::Cursor | ToolId::KimiCode) => {
                    entry.insert("url".into(), json!(url));
                }
                (Format::McpServers | Format::Vscode | Format::Crush, _) => {
                    entry.insert("type".into(), json!("http"));
                    entry.insert("url".into(), json!(url));
                }
                _ => {
                    entry.insert("url".into(), json!(url));
                }
            }
            if !headers.is_empty() {
                entry.insert("headers".into(), json!(headers));
            }
        }
    }
    Value::Object(entry)
}

/// Insert or replace a server entry in a parsed JSON config, preserving every
/// unrelated key.
pub(crate) fn write_json_entry(
    format: Format,
    tool: ToolId,
    doc: &mut Value,
    name: &str,
    transport: &Transport,
) -> Result<(), String> {
    let Value::Object(root) = doc else {
        return Err("config root is not a JSON object".into());
    };
    let key = format.servers_key().unwrap_or("mcpServers");
    if !root.contains_key(key) {
        root.insert(key.into(), Value::Object(Map::new()));
    }
    let Some(Value::Object(servers)) = root.get_mut(key) else {
        return Err(format!("`{key}` is not a JSON object"));
    };
    servers.insert(name.into(), build_json_entry(format, tool, transport));
    Ok(())
}

/// Remove a server entry from a parsed JSON config. Returns whether it existed.
pub(crate) fn remove_json_entry(format: Format, doc: &mut Value, name: &str) -> bool {
    let Some(key) = format.servers_key() else {
        return false;
    };
    let Some(Value::Object(servers)) = doc.get_mut(key) else {
        return false;
    };
    servers.remove(name).is_some()
}

/// Names of servers currently parked by their tool's disable flag.
pub(crate) fn json_disabled(flag: &DisableFlag, format: Format, doc: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(key) = format.servers_key() else {
        return out;
    };
    let Some(Value::Object(servers)) = doc.get(key) else {
        return out;
    };
    for (name, entry) in servers {
        if let Value::Object(obj) = entry {
            if obj.get(flag.key).and_then(Value::as_bool) == Some(flag.off_when) {
                out.insert(name.clone());
            }
        }
    }
    out
}

pub(crate) fn openclaw_disabled(flag: &DisableFlag, doc: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(Value::Object(servers)) = doc.get("mcp").and_then(|m| m.get("servers")) else {
        return out;
    };
    for (name, entry) in servers {
        if let Value::Object(obj) = entry {
            if obj.get(flag.key).and_then(Value::as_bool) == Some(flag.off_when) {
                out.insert(name.clone());
            }
        }
    }
    out
}
