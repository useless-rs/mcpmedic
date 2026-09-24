//! Codex TOML (`config.toml`) read/write: `[mcp_servers.*]` tables.
//!
//! Split out of [`crate::format`]. All edits preserve the rest of the
//! document (including comments) via `toml_edit`.

use std::collections::{BTreeMap, BTreeSet};

use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value as TomlValue};

use crate::model::{Servers, Transport};
use crate::registry::DisableFlag;

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

/// Apply provably safe repairs to a Codex `config.toml` document, in place:
/// insert missing `npx -y`. Returns a human-readable line per repair.
pub(crate) fn fix_toml_config(doc: &mut DocumentMut) -> Vec<String> {
    let mut fixes = Vec::new();
    let Some(item) = doc.get_mut("mcp_servers") else {
        return fixes;
    };
    let Some(table) = table_like_mut(item) else {
        return fixes;
    };
    for (name, entry) in table.iter_mut() {
        let table = match entry {
            Item::Table(t) => t as &mut dyn TableLike,
            Item::Value(TomlValue::InlineTable(t)) => t as &mut dyn TableLike,
            _ => continue,
        };
        let is_npx = table
            .get("command")
            .and_then(|i| i.as_value())
            .and_then(TomlValue::as_str)
            == Some("npx");
        if !is_npx {
            continue;
        }
        let Some(args) = table
            .get_mut("args")
            .and_then(|i| i.as_value_mut())
            .and_then(TomlValue::as_array_mut)
        else {
            continue;
        };
        let has_yes = args
            .iter()
            .any(|a| matches!(a.as_str(), Some("-y" | "--yes")));
        if !args.is_empty() && !has_yes {
            args.insert(0, "-y");
            fixes.push(format!(
                "server `{name}`: added `-y` to npx — auto-confirms the install prompt instead of hanging"
            ));
        }
    }
    fixes
}

fn table_like_mut(item: &mut Item) -> Option<&mut dyn TableLike> {
    match item {
        Item::Table(t) => Some(t),
        Item::Value(TomlValue::InlineTable(t)) => Some(t),
        Item::None | Item::Value(_) | Item::ArrayOfTables(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_inserts_missing_npx_yes() {
        let mut doc: DocumentMut = "[mcp_servers.svc]\ncommand = \"npx\"\nargs = [\"pkg\"]\n"
            .parse()
            .unwrap();
        let fixes = fix_toml_config(&mut doc);
        assert_eq!(fixes.len(), 1, "{fixes:?}");
        assert!(doc.to_string().contains("\"-y\""), "{}", doc);
        assert!(fix_toml_config(&mut doc).is_empty(), "second run is clean");
    }

    #[test]
    fn fix_skips_healthy_and_non_npx_entries() {
        let mut doc: DocumentMut = "[mcp_servers.ok]\ncommand = \"npx\"\nargs = [\"-y\", \"pkg\"]\n[mcp_servers.bun]\ncommand = \"bunx\"\nargs = [\"pkg\"]\n"
            .parse()
            .unwrap();
        assert!(fix_toml_config(&mut doc).is_empty());
    }
}
