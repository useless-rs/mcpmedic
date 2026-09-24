//! Per-format (de)serialization. Every tool's config is normalized into
//! [`model::Transport`] for reading, and rendered back into the tool's own
//! dialect for writing — never the other way around.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value as TomlValue};

use crate::model::{Servers, Transport};
use crate::registry::{DisableFlag, ToolId};

/// Config file dialects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    /// Top-level `mcpServers` key (Claude Code, Claude Desktop, Cursor,
    /// Windsurf, Gemini CLI, Cline, Roo Code).
    McpServers,
    /// VS Code user profile `mcp.json`: `servers` key with mandatory `type`.
    Vscode,
    /// Zed `settings.json`: `context_servers` key.
    Zed,
    /// Codex CLI `config.toml`: `[mcp_servers.<name>]` tables.
    CodexToml,
    /// opencode `opencode.json`: `mcp` object (read-only support).
    Opencode,
    /// Amp `settings.json`: flat `amp.mcpServers` key.
    Amp,
    /// Crush `crush.json`: `mcp` key with explicit `type` fields.
    Crush,
}

impl Format {
    /// JSON key that holds the server map, or `None` for non-JSON formats.
    fn servers_key(self) -> Option<&'static str> {
        match self {
            Self::McpServers => Some("mcpServers"),
            Self::Vscode => Some("servers"),
            Self::Zed => Some("context_servers"),
            Self::CodexToml | Self::Opencode => None,
            Self::Amp => Some("amp.mcpServers"),
            Self::Crush => Some("mcp"),
        }
    }
}

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

/// Remote URL field names across dialects, in preference order. Most tools
/// use `url`; Gemini CLI documents `httpUrl` for streamable HTTP and
/// Windsurf documents `serverUrl`.
const URL_FIELDS: [&str; 3] = ["url", "httpUrl", "serverUrl"];

/// The remote endpoint of an entry, whichever field carries it.
fn remote_url_of(obj: &Map<String, Value>) -> Option<String> {
    URL_FIELDS
        .iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_str).map(str::to_owned))
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

/// Extract a string map from a JSON object value, skipping non-string values.
fn string_map(v: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(map)) = v {
        for (k, val) in map {
            if let Value::String(s) = val {
                out.insert(k.clone(), s.clone());
            }
        }
    }
    out
}

/// Extract an array of strings, skipping non-string items.
fn string_array(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
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

/// Park or resume a server by writing its disable flag, preserving every
/// other field of the entry. Returns whether the named server exists.
pub(crate) fn set_json_disabled(
    flag: &DisableFlag,
    format: Format,
    doc: &mut Value,
    name: &str,
    off: bool,
) -> bool {
    let Some(key) = format.servers_key() else {
        return false;
    };
    let Some(Value::Object(servers)) = doc.get_mut(key) else {
        return false;
    };
    let Some(Value::Object(obj)) = servers.get_mut(name) else {
        return false;
    };
    let value = if off { flag.off_when } else { !flag.off_when };
    obj.insert(flag.key.into(), json!(value));
    true
}

/// Per-tool spelling of the remote-transport `type` field. `None` for tools
/// that infer the transport from the URL field (Cursor, Windsurf, Gemini CLI)
/// or handle it elsewhere (VS Code, Zed, Codex, opencode).
struct RemoteTypeRules {
    /// The value this tool documents for a streamable-HTTP server.
    expected: &'static str,
    /// Every spelling the tool accepts without breaking.
    accepted: &'static [&'static str],
}

fn remote_type_rules(tool: ToolId) -> Option<RemoteTypeRules> {
    match tool {
        ToolId::ClaudeCode | ToolId::ClaudeDesktop => Some(RemoteTypeRules {
            expected: "http",
            accepted: &["http", "streamable-http", "sse"],
        }),
        ToolId::RooCode => Some(RemoteTypeRules {
            expected: "streamable-http",
            accepted: &["streamable-http", "sse"],
        }),
        ToolId::Cline => Some(RemoteTypeRules {
            expected: "streamableHttp",
            accepted: &["streamableHttp", "sse"],
        }),
        _ => None,
    }
}

/// A pure-remote entry: a URL, no `command` — the only shape where `type`
/// fixes are provably safe. Hybrid command+url entries need human judgment.
fn is_pure_remote(obj: &Map<String, Value>) -> bool {
    remote_url_of(obj).is_some() && !obj.contains_key("command")
}

/// Scan a raw JSON document for issues that only exist per dialect, e.g. VS
/// Code requiring a `type` on every entry, or remote entries missing the
/// `type` spelling their tool can actually read.
pub(crate) fn json_raw_issues(
    tool: ToolId,
    format: Format,
    doc: &Value,
    disabled: &BTreeSet<String>,
) -> Vec<String> {
    let mut issues = Vec::new();
    let Some(key) = format.servers_key() else {
        return issues;
    };
    let Some(Value::Object(servers)) = doc.get(key) else {
        return issues;
    };
    for (name, entry) in servers {
        if name == "servers" {
            continue;
        }
        if disabled.contains(name) {
            continue;
        }
        let Value::Object(obj) = entry else {
            issues.push(format!("server `{name}`: entry is not a JSON object"));
            continue;
        };
        if format == Format::Vscode && !obj.contains_key("type") {
            issues.push(format!(
                "server `{name}`: VS Code requires a `type` field (`stdio` or `http`)"
            ));
        }
        if format == Format::McpServers {
            if let Some(rules) = remote_type_rules(tool) {
                let declared = obj.get("type").and_then(Value::as_str);
                if is_pure_remote(obj) {
                    match declared {
                        None => issues.push(format!(
                            "server `{name}`: has `url` but no `type` — {} requires `type: \"{}\"`",
                            tool.as_str(),
                            rules.expected
                        )),
                        Some(t) if !rules.accepted.contains(&t) => issues.push(format!(
                            "server `{name}`: `type: \"{t}\"` is not accepted by {} (expects `\"{}\"`)",
                            tool.as_str(),
                            rules.expected
                        )),
                        _ => {}
                    }
                }
                if declared.is_none() && is_pure_remote(obj) && obj.contains_key("transport") {
                    issues.push(format!(
                        "server `{name}`: uses `transport`, which {} ignores — the field is named `type`",
                        tool.as_str()
                    ));
                }
            }
        }
        for env_key in ["env", "environment"] {
            if let Some(Value::Object(env)) = obj.get(env_key) {
                for (k, v) in env {
                    if !v.is_string() {
                        issues.push(format!(
                            "server `{name}`: env value for `{k}` is not a string and will be ignored by the tool"
                        ));
                    }
                }
            }
        }
        if obj.contains_key("command")
            && matches!(obj.get("command"), Some(Value::Object(_)))
            && format == Format::Zed
        {
            issues.push(format!(
                "server `{name}`: uses Zed's legacy nested `command.path` layout"
            ));
        }
    }
    issues
}

/// Apply provably safe repairs to a parsed JSON config, in place. Returns a
/// human-readable line for each repair. Only ever adds or normalizes fields
/// the target tool reads — user data is never removed except `transport`,
/// which the tool ignores and which is moved into `type`.
fn fix_npx_yes(name: &str, entry: &mut Value, fixes: &mut Vec<String>) {
    let Some(obj) = entry.as_object_mut() else {
        return;
    };
    if obj.get("command").and_then(Value::as_str) != Some("npx") {
        return;
    }
    let needs_yes = obj
        .get("args")
        .and_then(Value::as_array)
        .is_some_and(|args| {
            !args.is_empty()
                && !args
                    .iter()
                    .any(|a| matches!(a.as_str(), Some("-y" | "--yes")))
        });
    if needs_yes {
        if let Some(args) = obj.get_mut("args").and_then(Value::as_array_mut) {
            args.insert(0, json!("-y"));
            fixes.push(format!(
                "server `{name}`: added `-y` to npx — auto-confirms the install prompt instead of hanging"
            ));
        }
    }
}

pub(crate) fn fix_json_config(tool: ToolId, format: Format, doc: &mut Value) -> Vec<String> {
    let mut fixes = Vec::new();
    let Some(key) = format.servers_key() else {
        return fixes;
    };
    let Some(Value::Object(servers)) = doc.get_mut(key) else {
        return fixes;
    };
    for (name, entry) in servers.iter_mut() {
        fix_npx_yes(name, entry, &mut fixes);
        let Value::Object(obj) = entry else {
            continue;
        };
        match format {
            Format::Vscode => {
                if !obj.contains_key("type") {
                    let kind = if obj.get("url").is_some() {
                        "http"
                    } else {
                        "stdio"
                    };
                    obj.insert("type".into(), json!(kind));
                    fixes.push(format!(
                        "server `{name}`: added missing `type: {kind}` (required by VS Code)"
                    ));
                }
            }
            Format::Zed => {
                let legacy = obj
                    .get("command")
                    .and_then(Value::as_object)
                    .and_then(|cmd| {
                        let path = cmd.get("path").and_then(Value::as_str)?;
                        let args = string_array(cmd.get("args").or_else(|| obj.get("args")));
                        let env = string_map(cmd.get("env").or_else(|| obj.get("env")));
                        Some((path.to_owned(), args, env))
                    });
                if let Some((path, args, env)) = legacy {
                    obj.insert("command".into(), json!(path));
                    if !args.is_empty() {
                        obj.insert("args".into(), json!(args));
                    }
                    if !env.is_empty() {
                        obj.insert("env".into(), json!(env));
                    }
                    fixes.push(format!(
                        "server `{name}`: migrated Zed's legacy nested `command` object to the flat layout"
                    ));
                }
            }
            Format::McpServers => {
                let Some(rules) = remote_type_rules(tool) else {
                    continue;
                };
                if !is_pure_remote(obj) {
                    continue;
                }
                let declared = obj.get("type").and_then(Value::as_str).map(str::to_owned);
                match declared.as_deref() {
                    None => {
                        let moved_transport = obj.remove("transport").is_some();
                        obj.insert("type".into(), json!(rules.expected));
                        let source = if moved_transport {
                            format!("moved `transport` to `type: \"{}\"`", rules.expected)
                        } else {
                            format!(
                                "added `type: \"{}\"` ({tool} skips entries without it)",
                                rules.expected,
                                tool = tool.as_str()
                            )
                        };
                        fixes.push(format!("server `{name}`: {source}"));
                    }
                    Some(t) if !rules.accepted.contains(&t) => {
                        obj.insert("type".into(), json!(rules.expected));
                        fixes.push(format!(
                            "server `{name}`: corrected `type: \"{t}\"` → `\"{}\"`",
                            rules.expected
                        ));
                    }
                    _ => {}
                }
            }
            Format::CodexToml | Format::Opencode | Format::Amp | Format::Crush => {}
        }
    }
    fixes
}

// ---------------------------------------------------------------------------
// Codex TOML
// ---------------------------------------------------------------------------

/// Read `[mcp_servers.*]` tables out of a Codex `config.toml` document.
pub(crate) fn toml_servers(doc: &DocumentMut) -> (Servers, Vec<String>) {
    let mut servers = Servers::new();
    let mut problems = Vec::new();
    let Some(item) = doc.get("mcp_servers") else {
        return (servers, problems);
    };
    let Some(table) = table_like(item) else {
        problems.push("`mcp_servers` is not a table".into());
        return (servers, problems);
    };
    for (name, entry) in table.iter() {
        let Some(entry) = table_like(entry) else {
            problems.push(format!("server `{name}`: entry is not a table"));
            continue;
        };
        if let Some(url) = tbl_str(entry, "url") {
            let headers = tbl_map(entry, "http_headers");
            servers.insert(name.to_string(), Transport::Remote { url, headers });
        } else if let Some(command) = tbl_str(entry, "command") {
            let args = tbl_args(entry);
            let env = tbl_map(entry, "env");
            servers.insert(name.to_string(), Transport::Stdio { command, args, env });
        } else {
            problems.push(format!(
                "server `{name}`: entry has neither `command` nor `url`"
            ));
        }
    }
    (servers, problems)
}

fn tbl_str(table: &dyn TableLike, key: &str) -> Option<String> {
    table
        .get(key)?
        .as_value()
        .and_then(TomlValue::as_str)
        .map(str::to_owned)
}

fn tbl_args(table: &dyn TableLike) -> Vec<String> {
    table
        .get("args")
        .and_then(|item| item.as_value())
        .and_then(TomlValue::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn tbl_map(table: &dyn TableLike, key: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(sub) = table.get(key).and_then(table_like) {
        for (k, v) in sub.iter() {
            if let Some(s) = v.as_value().and_then(TomlValue::as_str) {
                out.insert(k.to_string(), s.to_owned());
            }
        }
    }
    out
}

fn table_like(item: &Item) -> Option<&dyn TableLike> {
    match item {
        Item::Table(t) => Some(t),
        Item::Value(TomlValue::InlineTable(t)) => Some(t),
        Item::None | Item::Value(_) | Item::ArrayOfTables(_) => None,
    }
}

/// Insert or replace a `[mcp_servers.<name>]` table, preserving the rest of
/// the document (including comments) via `toml_edit`.
pub(crate) fn write_toml_entry(
    doc: &mut DocumentMut,
    name: &str,
    transport: &Transport,
) -> Result<(), String> {
    let root = doc.as_table_mut();
    if !root.contains_key("mcp_servers") {
        let mut tbl = Table::new();
        tbl.set_implicit(true);
        root.insert("mcp_servers", Item::Table(tbl));
    }
    let Some(Item::Table(servers)) = root.get_mut("mcp_servers") else {
        return Err("`mcp_servers` is an inline table; refusing to edit it".into());
    };

    let mut entry = Table::new();
    match transport {
        Transport::Stdio { command, args, env } => {
            entry.insert("command", Item::Value(TomlValue::from(command.clone())));
            if !args.is_empty() {
                let mut arr = Array::new();
                for a in args {
                    arr.push(a.as_str());
                }
                entry.insert("args", Item::Value(TomlValue::Array(arr)));
            }
            if !env.is_empty() {
                let mut env_tbl = Table::new();
                for (k, v) in env {
                    env_tbl.insert(k.as_str(), Item::Value(TomlValue::from(v.clone())));
                }
                entry.insert("env", Item::Table(env_tbl));
            }
        }
        Transport::Remote { url, headers } => {
            entry.insert("url", Item::Value(TomlValue::from(url.clone())));
            if !headers.is_empty() {
                let mut h = Table::new();
                for (k, v) in headers {
                    h.insert(k.as_str(), Item::Value(TomlValue::from(v.clone())));
                }
                entry.insert("http_headers", Item::Table(h));
            }
        }
    }
    servers.insert(name, Item::Table(entry));
    Ok(())
}

/// Remove a `[mcp_servers.<name>]` table. Returns whether it existed.
pub(crate) fn remove_toml_entry(doc: &mut DocumentMut, name: &str) -> Result<bool, String> {
    let root = doc.as_table_mut();
    let Some(servers) = root.get_mut("mcp_servers") else {
        return Ok(false);
    };
    if let Item::Table(tbl) = servers {
        Ok(tbl.remove(name).is_some())
    } else {
        Err("`mcp_servers` is an inline table; refusing to edit it".into())
    }
}

/// Names of `[mcp_servers.<name>]` tables parked by the tool's disable flag.
pub(crate) fn toml_disabled(flag: &DisableFlag, doc: &DocumentMut) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(item) = doc.get("mcp_servers") else {
        return out;
    };
    let Some(table) = table_like(item) else {
        return out;
    };
    for (name, entry) in table.iter() {
        if let Some(entry) = table_like(entry) {
            let parked = entry
                .get(flag.key)
                .and_then(|item| item.as_value())
                .and_then(TomlValue::as_bool)
                == Some(flag.off_when);
            if parked {
                out.insert(name.to_string());
            }
        }
    }
    out
}

/// Park or resume a server by writing its disable flag, preserving the rest
/// of the document (including comments). Returns whether the server exists.
pub(crate) fn set_toml_disabled(
    flag: &DisableFlag,
    doc: &mut DocumentMut,
    name: &str,
    off: bool,
) -> Result<bool, String> {
    let root = doc.as_table_mut();
    let Some(servers) = root.get_mut("mcp_servers") else {
        return Ok(false);
    };
    let Item::Table(tbl) = servers else {
        return Err("`mcp_servers` is an inline table; refusing to edit it".into());
    };
    let Some(entry) = tbl.get_mut(name) else {
        return Ok(false);
    };
    let value = if off { flag.off_when } else { !flag.off_when };
    match entry {
        Item::Table(t) => {
            t.insert(flag.key, Item::Value(TomlValue::from(value)));
            Ok(true)
        }
        Item::Value(TomlValue::InlineTable(t)) => {
            t.insert(flag.key, TomlValue::from(value));
            Ok(true)
        }
        _ => Err(format!("server `{name}`: entry is not a table")),
    }
}

// ---------------------------------------------------------------------------
// JSONC tolerance
// ---------------------------------------------------------------------------

/// Strip `//`, `/* */` comments and trailing commas so a JSONC file can be
/// parsed read-only. String literals are preserved exactly.
pub(crate) fn jsonc_strip(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut out = String::with_capacity(source.len());
    let mut in_string = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(n) = next {
                    out.push(n);
                    i += 2;
                    continue;
                }
            } else if c == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
                i += 1;
            }
            '/' if next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    i += 1;
                }
                i = (i + 2).min(chars.len());
            }
            ',' => {
                // Drop the comma if only whitespace separates it from `}` or `]`.
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                if matches!(chars.get(j), Some('}' | ']')) {
                    i += 1; // skip comma, whitespace is copied naturally below
                } else {
                    out.push(c);
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(command: &str, args: &[&str]) -> Transport {
        Transport::Stdio {
            command: command.into(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn parses_plain_stdio_entry() {
        let v = json!({"command": "npx", "args": ["-y", "pkg"], "env": {"K": "V"}});
        let t = parse_json_entry(&v).unwrap();
        assert_eq!(t.kind(), "stdio");
        let Transport::Stdio { command, args, env } = t else {
            panic!("expected stdio");
        };
        assert_eq!(command, "npx");
        assert_eq!(args, vec!["-y", "pkg"]);
        assert_eq!(env.get("K").map(String::as_str), Some("V"));
    }

    #[test]
    fn parses_remote_entry_with_headers() {
        let v = json!({"type": "http", "url": "https://example.com", "headers": {"A": "b"}});
        let Transport::Remote { url, headers } = parse_json_entry(&v).unwrap() else {
            panic!("expected remote");
        };
        assert_eq!(url, "https://example.com");
        assert_eq!(headers.get("A").map(String::as_str), Some("b"));
    }

    #[test]
    fn parses_opencode_command_array() {
        let v =
            json!({"type": "local", "command": ["npx", "-y", "pkg"], "environment": {"E": "1"}});
        let Transport::Stdio { command, args, env } = parse_json_entry(&v).unwrap() else {
            panic!("expected stdio");
        };
        assert_eq!(command, "npx");
        assert_eq!(args, vec!["-y", "pkg"]);
        assert_eq!(env.get("E").map(String::as_str), Some("1"));
    }

    #[test]
    fn parses_zed_legacy_nested_command() {
        let v = json!({"command": {"path": "docker", "args": ["run"]}, "env": {}});
        let Transport::Stdio { command, args, .. } = parse_json_entry(&v).unwrap() else {
            panic!("expected stdio");
        };
        assert_eq!(command, "docker");
        assert_eq!(args, vec!["run"]);
    }

    #[test]
    fn rejects_http_type_without_url() {
        let v = json!({"type": "http", "command": "x"});
        assert!(parse_json_entry(&v).is_err());
    }

    #[test]
    fn rejects_entry_without_command_or_url() {
        let v = json!({"env": {}});
        assert!(parse_json_entry(&v).is_err());
    }

    #[test]
    fn write_json_entry_preserves_unrelated_keys() {
        let mut doc = json!({"mcpServers": {"old": {"command": "true"}}, "numStartups": 42});
        write_json_entry(
            Format::McpServers,
            ToolId::ClaudeCode,
            &mut doc,
            "new",
            &stdio("npx", &["-y", "p"]),
        )
        .unwrap();
        assert_eq!(doc["numStartups"], json!(42));
        assert_eq!(doc["mcpServers"]["old"]["command"], json!("true"));
        assert_eq!(doc["mcpServers"]["new"]["command"], json!("npx"));
        assert_eq!(doc["mcpServers"]["new"]["args"], json!(["-y", "p"]));
        // VS Code dialect gets a type field.
        let mut vscode_doc = json!({});
        write_json_entry(
            Format::Vscode,
            ToolId::Vscode,
            &mut vscode_doc,
            "x",
            &stdio("npx", &[]),
        )
        .unwrap();
        assert_eq!(vscode_doc["servers"]["x"]["type"], json!("stdio"));
    }

    #[test]
    fn parses_amp_and_crush_formats() {
        let amp = json!({"amp.mcpServers": {"src": {"command": "npx", "args": ["-y", "x"]}}});
        let (servers, issues) = json_servers(Format::Amp, &amp);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(servers.len(), 1);
        let Some(Transport::Stdio { command, .. }) = servers.get("src") else {
            panic!("expected stdio: {servers:?}");
        };
        assert_eq!(command, "npx");

        let crush = json!({"mcp": {"github": {"type": "http", "url": "https://x/mcp"}}});
        let (servers, issues) = json_servers(Format::Crush, &crush);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(servers.len(), 1);
        let Some(Transport::Remote { url, .. }) = servers.get("github") else {
            panic!("expected remote: {servers:?}");
        };
        assert_eq!(url, "https://x/mcp");

        let entry = build_json_entry(
            Format::Crush,
            ToolId::Crush,
            &Transport::Stdio {
                command: "node".into(),
                args: vec!["s.js".into()],
                env: BTreeMap::new(),
            },
        );
        assert_eq!(entry["type"], "stdio");
        assert_eq!(entry["command"], "node");

        let entry = build_json_entry(
            Format::Amp,
            ToolId::Amp,
            &Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "x".into()],
                env: BTreeMap::new(),
            },
        );
        assert!(
            entry.get("type").is_none(),
            "Amp stdio has no type: {entry}"
        );
    }

    #[test]
    fn parses_gemini_httpurl_and_windsurf_serverurl() {
        let gemini = json!({"httpUrl": "https://x/mcp", "headers": {"Authorization": "Bearer t"}});
        let Transport::Remote { url, headers } = parse_json_entry(&gemini).unwrap() else {
            panic!("expected remote");
        };
        assert_eq!(url, "https://x/mcp");
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer t")
        );

        let windsurf = json!({"serverUrl": "https://y/mcp"});
        let Transport::Remote { url, .. } = parse_json_entry(&windsurf).unwrap() else {
            panic!("expected remote");
        };
        assert_eq!(url, "https://y/mcp");
    }

    #[test]
    fn disable_flags_roundtrip_in_json() {
        let roo = DisableFlag {
            key: "disabled",
            off_when: true,
        };
        let mut doc = json!({"mcpServers": {"x": {"command": "n", "disabled": true}}});
        let parked: BTreeSet<String> = ["x".to_owned()].into_iter().collect();
        assert_eq!(json_disabled(&roo, Format::McpServers, &doc), parked);
        assert!(set_json_disabled(
            &roo,
            Format::McpServers,
            &mut doc,
            "x",
            false
        ));
        assert_eq!(doc["mcpServers"]["x"]["disabled"], json!(false));
        assert!(json_disabled(&roo, Format::McpServers, &doc).is_empty());
        assert!(set_json_disabled(
            &roo,
            Format::McpServers,
            &mut doc,
            "x",
            true
        ));
        assert_eq!(doc["mcpServers"]["x"]["disabled"], json!(true));
        assert!(!set_json_disabled(
            &roo,
            Format::McpServers,
            &mut doc,
            "missing",
            true
        ));

        let zed = DisableFlag {
            key: "enabled",
            off_when: false,
        };
        let mut doc = json!({"context_servers": {"y": {"command": "n"}}});
        assert!(json_disabled(&zed, Format::Zed, &doc).is_empty());
        assert!(set_json_disabled(&zed, Format::Zed, &mut doc, "y", true));
        assert_eq!(doc["context_servers"]["y"]["enabled"], json!(false));
        assert_eq!(
            json_disabled(&zed, Format::Zed, &doc),
            ["y".to_owned()].into_iter().collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn disable_flags_roundtrip_in_toml_preserving_comments() {
        let codex = DisableFlag {
            key: "enabled",
            off_when: false,
        };
        let src = "# keep me\nmodel = \"gpt\"\n\n[mcp_servers.svc]\ncommand = \"node\"\n";
        let mut doc: DocumentMut = src.parse().unwrap();
        assert!(toml_disabled(&codex, &doc).is_empty());
        assert!(set_toml_disabled(&codex, &mut doc, "svc", true).unwrap());
        let out = doc.to_string();
        assert!(out.contains("enabled = false"));
        assert!(out.contains("# keep me"));
        assert_eq!(
            toml_disabled(&codex, &doc),
            ["svc".to_owned()].into_iter().collect::<BTreeSet<_>>()
        );
        assert!(set_toml_disabled(&codex, &mut doc, "svc", false).unwrap());
        assert!(doc.to_string().contains("enabled = true"));
        assert!(!set_toml_disabled(&codex, &mut doc, "missing", true).unwrap());
    }

    #[test]
    fn raw_issues_skip_parked_servers() {
        let mut parked = BTreeSet::new();
        parked.insert("parked".to_owned());
        let doc = json!({
            "mcpServers": {
                "parked": {"url": "https://x"},
                "live": {"url": "https://x"}
            }
        });
        let issues = json_raw_issues(ToolId::ClaudeCode, Format::McpServers, &doc, &parked);
        assert_eq!(issues.len(), 1);
        assert!(
            issues[0].contains("live"),
            "only the live entry is flagged: {issues:?}"
        );
    }

    #[test]
    fn remote_entries_are_written_in_each_tools_dialect() {
        let remote = Transport::Remote {
            url: "https://x/mcp".into(),
            headers: [("Authorization".to_owned(), "Bearer t".to_owned())].into(),
        };

        let claude = build_json_entry(Format::McpServers, ToolId::ClaudeCode, &remote);
        assert_eq!(claude["type"], json!("http"));
        assert_eq!(claude["url"], json!("https://x/mcp"));
        assert_eq!(claude["headers"]["Authorization"], json!("Bearer t"));

        let cursor = build_json_entry(Format::McpServers, ToolId::Cursor, &remote);
        assert!(cursor.get("type").is_none(), "cursor infers transport");
        assert_eq!(cursor["url"], json!("https://x/mcp"));

        let windsurf = build_json_entry(Format::McpServers, ToolId::Windsurf, &remote);
        assert!(windsurf.get("url").is_none(), "windsurf uses serverUrl");
        assert!(windsurf.get("type").is_none());
        assert_eq!(windsurf["serverUrl"], json!("https://x/mcp"));

        let gemini = build_json_entry(Format::McpServers, ToolId::GeminiCli, &remote);
        assert!(gemini.get("url").is_none(), "gemini uses httpUrl");
        assert!(gemini.get("type").is_none());
        assert_eq!(gemini["httpUrl"], json!("https://x/mcp"));
        assert_eq!(gemini["headers"]["Authorization"], json!("Bearer t"));

        let roo = build_json_entry(Format::McpServers, ToolId::RooCode, &remote);
        assert_eq!(roo["type"], json!("streamable-http"));

        let cline = build_json_entry(Format::McpServers, ToolId::Cline, &remote);
        assert_eq!(cline["type"], json!("streamableHttp"));

        let zed = build_json_entry(Format::Zed, ToolId::Zed, &remote);
        assert!(zed.get("type").is_none());
        assert_eq!(zed["url"], json!("https://x/mcp"));

        for (tool, entry) in [
            (ToolId::ClaudeCode, claude),
            (ToolId::Cursor, cursor),
            (ToolId::Windsurf, windsurf),
            (ToolId::GeminiCli, gemini),
            (ToolId::RooCode, roo),
            (ToolId::Cline, cline),
        ] {
            let doc = json!({"mcpServers": {"written": entry}});
            assert!(
                json_raw_issues(tool, Format::McpServers, &doc, &BTreeSet::new()).is_empty(),
                "doctor must not flag the dialect mcpmedic writes"
            );
        }
    }

    #[test]
    fn remove_json_entry_only_removes_named() {
        let mut doc = json!({"mcpServers": {"a": {"command": "x"}, "b": {"command": "y"}}});
        assert!(remove_json_entry(Format::McpServers, &mut doc, "a"));
        assert!(!remove_json_entry(Format::McpServers, &mut doc, "a"));
        assert!(doc["mcpServers"].get("b").is_some());
    }

    #[test]
    fn vscode_raw_issues_flags_missing_type() {
        let doc =
            json!({"servers": {"nope": {"command": "x"}, "ok": {"type": "stdio", "command": "y"}}});
        let issues = json_raw_issues(ToolId::Vscode, Format::Vscode, &doc, &BTreeSet::new());
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("nope"));
    }

    #[test]
    fn raw_issues_flag_remote_type_dialects() {
        let claude = json!({"mcpServers": {"r": {"url": "https://x"}}});
        let issues = json_raw_issues(
            ToolId::ClaudeCode,
            Format::McpServers,
            &claude,
            &BTreeSet::new(),
        );
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains("requires `type: \"http\"`"));

        let roo = json!({"mcpServers": {"r": {"url": "https://x", "type": "http"}}});
        let issues = json_raw_issues(ToolId::RooCode, Format::McpServers, &roo, &BTreeSet::new());
        assert!(issues[0].contains("not accepted"));

        let transport = json!({"mcpServers": {"r": {"url": "https://x", "transport": "http"}}});
        let issues = json_raw_issues(
            ToolId::ClaudeCode,
            Format::McpServers,
            &transport,
            &BTreeSet::new(),
        );
        assert!(issues.iter().any(|i| i.contains("ignores")));

        let cursor = json!({"mcpServers": {"r": {"url": "https://x"}}});
        assert!(
            json_raw_issues(
                ToolId::Cursor,
                Format::McpServers,
                &cursor,
                &BTreeSet::new()
            )
            .is_empty()
        );

        let hybrid = json!({"mcpServers": {"r": {"command": "x", "url": "https://x"}}});
        assert!(
            json_raw_issues(
                ToolId::ClaudeCode,
                Format::McpServers,
                &hybrid,
                &BTreeSet::new()
            )
            .is_empty()
        );
    }

    #[test]
    fn fix_adds_npx_yes_flag() {
        let mut doc = json!({"mcpServers": {
            "mem": {"command": "npx", "args": ["@modelcontextprotocol/server-memory"]},
            "safe": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-memory"]},
            "other": {"command": "uvx", "args": ["mcp-server-fetch"]}
        }});
        let fixes = fix_json_config(ToolId::Cursor, Format::McpServers, &mut doc);
        assert_eq!(fixes.len(), 1, "{fixes:?}");
        assert!(fixes[0].contains("`mem`"), "{fixes:?}");
        assert_eq!(doc["mcpServers"]["mem"]["args"][0], "-y");
        assert_eq!(
            doc["mcpServers"]["mem"]["args"][1],
            "@modelcontextprotocol/server-memory"
        );
        assert_eq!(doc["mcpServers"]["safe"]["args"][0], "-y");
        assert_eq!(
            doc["mcpServers"]["safe"]["args"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            doc["mcpServers"]["other"]["args"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn fix_repairs_remote_type_dialects_per_tool() {
        let mut claude = json!({"mcpServers": {"r": {"url": "https://x", "headers": {"A": "b"}}}});
        let fixes = fix_json_config(ToolId::ClaudeCode, Format::McpServers, &mut claude);
        assert_eq!(fixes.len(), 1);
        assert_eq!(claude["mcpServers"]["r"]["type"], json!("http"));
        assert_eq!(claude["mcpServers"]["r"]["headers"]["A"], json!("b"));

        let mut roo = json!({"mcpServers": {"r": {"url": "https://x", "type": "http"}}});
        fix_json_config(ToolId::RooCode, Format::McpServers, &mut roo);
        assert_eq!(roo["mcpServers"]["r"]["type"], json!("streamable-http"));

        let mut cline = json!({"mcpServers": {"r": {"url": "https://x", "transport": "http"}}});
        fix_json_config(ToolId::Cline, Format::McpServers, &mut cline);
        assert_eq!(cline["mcpServers"]["r"]["type"], json!("streamableHttp"));
        assert!(cline["mcpServers"]["r"].get("transport").is_none());

        let mut hybrid = json!({"mcpServers": {"r": {"command": "x", "url": "https://x"}}});
        assert!(fix_json_config(ToolId::ClaudeCode, Format::McpServers, &mut hybrid).is_empty());
        assert!(
            hybrid["mcpServers"]["r"].get("type").is_none(),
            "hybrid entries must never be auto-fixed"
        );
    }

    #[test]
    fn fix_adds_vscode_type_by_shape() {
        let mut doc = json!({
            "servers": {
                "s": {"command": "x"},
                "r": {"url": "https://x"},
                "ok": {"type": "http", "url": "https://y"}
            },
            "inputs": []
        });
        let fixes = fix_json_config(ToolId::Vscode, Format::Vscode, &mut doc);
        assert_eq!(fixes.len(), 2);
        assert_eq!(doc["servers"]["s"]["type"], json!("stdio"));
        assert_eq!(doc["servers"]["r"]["type"], json!("http"));
        assert_eq!(doc["inputs"], json!([]));
    }

    #[test]
    fn fix_migrates_zed_legacy_layout() {
        let mut doc = json!({
            "theme": "One Dark",
            "context_servers": {
                "old": {"command": {"path": "sh", "args": ["-c"]}, "env": {"K": "V"}}
            }
        });
        let fixes = fix_json_config(ToolId::Zed, Format::Zed, &mut doc);
        assert_eq!(fixes.len(), 1);
        assert_eq!(doc["context_servers"]["old"]["command"], json!("sh"));
        assert_eq!(doc["context_servers"]["old"]["args"], json!(["-c"]));
        assert_eq!(doc["context_servers"]["old"]["env"]["K"], json!("V"));
        assert_eq!(doc["theme"], json!("One Dark"));
    }

    #[test]
    fn jsonc_strip_removes_comments_and_trailing_commas() {
        let src = r#"{
            // a comment
            "a": "b/*not a comment*/", /* block */
            "c": 1,
        }"#;
        let stripped = jsonc_strip(src);
        let parsed: Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(parsed["a"], json!("b/*not a comment*/"));
        assert_eq!(parsed["c"], json!(1));
    }

    #[test]
    fn jsonc_strip_leaves_clean_json_untouched() {
        let src = r#"{"a": [1, 2], "b": {"c": "d"}}"#;
        assert_eq!(jsonc_strip(src), src);
    }

    #[test]
    fn toml_roundtrip_preserves_other_settings() {
        let src = r#"
# my profile comment
model = "gpt-5"

[mcp_servers.keep]
command = "node"
args = ["server.js"]
"#;
        let mut doc: DocumentMut = src.parse().unwrap();
        let (servers, problems) = toml_servers(&doc);
        assert!(problems.is_empty());
        assert_eq!(servers.len(), 1);

        write_toml_entry(&mut doc, "added", &stdio("npx", &["-y", "pkg"])).unwrap();
        assert!(remove_toml_entry(&mut doc, "keep").unwrap());

        let out = doc.to_string();
        assert!(out.contains("# my profile comment"));
        assert!(out.contains("model = \"gpt-5\""));
        assert!(out.contains("[mcp_servers.added]"));
        assert!(out.contains("command = \"npx\""));
        assert!(!out.contains("[mcp_servers.keep]"));

        let reparsed: DocumentMut = out.parse().unwrap();
        let (servers2, _) = toml_servers(&reparsed);
        assert_eq!(servers2.len(), 1);
        assert!(servers2.contains_key("added"));
    }

    #[test]
    fn opencode_reads_flat_and_nested() {
        let doc = json!({
            "mcp": {
                "flat-remote": {"type": "remote", "url": "https://example.com"},
                "servers": {
                    "nested-local": {"type": "local", "command": ["bun", "x", "srv"]}
                }
            }
        });
        let (servers, problems) = opencode_servers(&doc);
        assert!(problems.is_empty());
        assert_eq!(servers.len(), 2);
        assert!(servers.contains_key("flat-remote"));
        assert!(servers.contains_key("nested-local"));
    }
}
