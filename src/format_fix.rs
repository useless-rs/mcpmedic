//! JSON diagnose + repair: per-dialect raw issues (`doctor`), provably safe
//! auto-fixes (`doctor --fix`), and the disable-flag writer.
//!
//! Split out of [`crate::format`]. Repairs only ever add or normalize fields
//! the target tool reads — user data is never removed except `transport`,
//! which the tool ignores and which is moved into `type`.

use std::collections::BTreeSet;

use serde_json::{Map, Value, json};

use crate::format::{Format, remote_url_of, string_array, string_map};
use crate::registry::{DisableFlag, ToolId};

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
            Format::CodexToml
            | Format::Opencode
            | Format::Amp
            | Format::Crush
            | Format::OpenClaw
            | Format::Yaml => {}
        }
    }
    fixes
}
