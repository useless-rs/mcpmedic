//! Per-format (de)serialization. Every tool's config is normalized into
//! [`model::Transport`] for reading, and rendered back into the tool's own
//! dialect for writing — never the other way around.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

#[cfg(test)]
use toml_edit::DocumentMut;

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
    /// `OpenClaw` `openclaw.json`: JSON5, nested `mcp.servers` (read-only).
    OpenClaw,
    /// YAML configs (Goose, Continue config.yaml): parsed by the yaml
    /// module, then extracted per tool from list-shaped sections.
    Yaml,
}

impl Format {
    /// JSON key that holds the server map, or `None` for non-JSON formats.
    pub(crate) fn servers_key(self) -> Option<&'static str> {
        match self {
            Self::McpServers => Some("mcpServers"),
            Self::Vscode => Some("servers"),
            Self::Zed => Some("context_servers"),
            Self::CodexToml | Self::Opencode | Self::OpenClaw | Self::Yaml => None,
            Self::Amp => Some("amp.mcpServers"),
            Self::Crush => Some("mcp"),
        }
    }
}

/// Remote URL field names across dialects, in preference order. Most tools
/// use `url`; Gemini CLI documents `httpUrl` for streamable HTTP and
/// Windsurf documents `serverUrl`.
const URL_FIELDS: [&str; 3] = ["url", "httpUrl", "serverUrl"];

/// The remote endpoint of an entry, whichever field carries it.
pub(crate) fn remote_url_of(obj: &Map<String, Value>) -> Option<String> {
    URL_FIELDS
        .iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_str).map(str::to_owned))
}

/// Extract a string map from a JSON object value, skipping non-string values.
pub(crate) fn string_map(v: Option<&Value>) -> BTreeMap<String, String> {
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
pub(crate) fn string_array(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// JSONC tolerance — see `format_jsonc`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;
    use crate::format_fix::{fix_json_config, json_raw_issues, set_json_disabled};
    use crate::format_json::{
        build_json_entry, json_disabled, json_servers, openclaw_disabled, openclaw_servers,
        opencode_servers, parse_json_entry, remove_json_entry, write_json_entry,
    };
    use crate::format_jsonc::jsonc_strip;
    use crate::format_toml::{
        remove_toml_entry, set_toml_disabled, toml_disabled, toml_servers, write_toml_entry,
    };
    use crate::model::Transport;
    use crate::registry::{DisableFlag, ToolId};

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
    fn openclaw_reads_nested_mcp_servers() {
        let doc = json!({
            "mcp": {
                "servers": {
                    "docs": {"command": "uvx", "args": ["mcp-server-fetch"]},
                    "remote": {
                        "url": "https://example.com/mcp",
                        "transport": "streamable-http"
                    },
                    "parked": {"command": "npx", "args": ["-y", "x"], "enabled": false}
                }
            }
        });
        let (servers, problems) = openclaw_servers(&doc);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(servers.len(), 3);
        assert!(servers.contains_key("docs"));
        assert!(servers.contains_key("remote"));
        let flag = DisableFlag {
            key: "enabled",
            off_when: false,
        };
        let disabled = openclaw_disabled(&flag, &doc);
        assert!(disabled.contains("parked"), "{disabled:?}");
        assert!(!disabled.contains("docs"), "{disabled:?}");
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
