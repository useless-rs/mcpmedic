//! YAML bridge: extract servers from list-shaped YAML sections (Goose
//! `extensions`, Continue `config.yaml` `mcpServers`) by normalizing items
//! into the shape [`crate::format_json::parse_json_entry`] understands.
//!
//! Split out of [`crate::format`].

use std::collections::BTreeSet;

use serde_json::{Map, Value};

use crate::format_json::parse_json_entry;
use crate::model::Servers;
use crate::registry::{DisableFlag, ToolId};

/// Extract servers from a list-shaped YAML section (Goose extensions,
/// Continue config.yaml mcpServers): each list item is an object keyed
/// by its `name` field.
pub(crate) fn yaml_servers(tool: ToolId, doc: &Value) -> (Servers, Vec<String>) {
    let list = match tool {
        ToolId::Goose => doc.get("extensions").and_then(Value::as_array),
        ToolId::ContinueYaml => doc.get("mcpServers").and_then(Value::as_array),
        _ => None,
    };
    let mut servers = Servers::new();
    let mut problems = Vec::new();
    for (i, item) in list.into_iter().flatten().enumerate() {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            problems.push(format!("yaml entry {i} has no `name`"));
            continue;
        };
        let parsed = match tool {
            ToolId::Goose => parse_json_entry(&normalize_goose(item)),
            _ => parse_json_entry(item),
        };
        match parsed {
            Ok(t) => {
                servers.insert(name.to_string(), t);
            }
            Err(e) => problems.push(format!("`{name}`: {e}")),
        }
    }
    (servers, problems)
}

/// Goose nests the entry under `transport` with `env` on the item;
/// flatten both into the shape the JSON entry parser understands.
fn normalize_goose(item: &Value) -> Value {
    let mut map = Map::new();
    if let Some(t) = item.get("transport") {
        for (k, v) in t.as_object().into_iter().flatten() {
            map.insert(k.clone(), v.clone());
        }
    }
    if let Some(env) = item.get("env") {
        map.insert("env".into(), env.clone());
    }
    Value::Object(map)
}

pub(crate) fn yaml_disabled(tool: ToolId, flag: &DisableFlag, doc: &Value) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let list = match tool {
        ToolId::Goose => doc.get("extensions").and_then(Value::as_array),
        _ => None,
    };
    for item in list.into_iter().flatten() {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        if item.get(flag.key).and_then(Value::as_bool) == Some(flag.off_when) {
            out.insert(name.to_string());
        }
    }
    out
}
