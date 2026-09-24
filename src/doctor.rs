//! Health checks: broken JSON, dead commands, cross-tool drift, dialect
//! violations and context-window bloat.

use std::collections::BTreeMap;
use std::path::Path;

use crate::doctor_env::{command_exists, extract_env_var_refs};
use crate::format::Format;
use crate::model::Transport;
use crate::registry::ToolId;
use crate::store::{ConfigState, LoadedConfig};

/// Severity of a single doctor finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Severity {
    /// The server almost certainly does not work.
    Critical,
    /// It works today but will surprise you soon (drift, odd values).
    Warning,
    /// Worth knowing, not a defect.
    Info,
}

/// One doctor finding.
#[derive(Clone, Debug)]
pub(crate) struct Finding {
    /// Severity level.
    pub severity: Severity,
    /// Tool the finding belongs to.
    pub tool: ToolId,
    /// Server name when the finding is about a specific server.
    pub server: Option<String>,
    /// Human-readable explanation.
    pub message: String,
}

/// A tool with its loaded state, ready for doctoring.
pub(crate) struct ToolLoad {
    /// Tool id.
    pub id: ToolId,
    /// Config format.
    pub format: Format,
    /// Load outcome.
    pub state: ConfigState,
}

impl ToolLoad {
    /// Convenience constructor.
    pub(crate) fn new(id: ToolId, format: Format, state: ConfigState) -> Self {
        Self { id, format, state }
    }
}

/// Run every health check across the loaded tools.
pub(crate) fn diagnose(tools: &[ToolLoad], home: &Path, path_env: &str) -> Vec<Finding> {
    let mut findings = Vec::new();

    for tool in tools {
        match &tool.state {
            ConfigState::Missing => {}
            ConfigState::ParseError(msg) => findings.push(Finding {
                severity: Severity::Critical,
                tool: tool.id,
                server: None,
                message: format!("config cannot be parsed: {msg}"),
            }),
            ConfigState::Loaded(cfg) => {
                diagnose_loaded(&mut findings, tool, cfg, home, path_env);
            }
        }
    }

    findings.extend(drift_findings(tools));
    findings
}

const CONTEXT_BLOAT_THRESHOLD: usize = 15;

fn diagnose_loaded(
    findings: &mut Vec<Finding>,
    tool: &ToolLoad,
    cfg: &LoadedConfig,
    home: &Path,
    path_env: &str,
) {
    for problem in &cfg.problems {
        findings.push(Finding {
            severity: Severity::Critical,
            tool: tool.id,
            server: None,
            message: problem.clone(),
        });
    }

    if let crate::store::RawDoc::Json(doc) = &cfg.raw {
        for issue in crate::format_fix::json_raw_issues(tool.id, tool.format, doc, &cfg.disabled) {
            let severity = if issue.contains("legacy") || issue.contains("ignored") {
                Severity::Warning
            } else {
                Severity::Critical
            };
            findings.push(Finding {
                severity,
                tool: tool.id,
                server: None,
                message: issue,
            });
        }
    }

    for (name, transport) in &cfg.servers {
        if name.trim().is_empty() {
            findings.push(Finding {
                severity: Severity::Warning,
                tool: tool.id,
                server: Some(name.clone()),
                message: "server name is empty or whitespace".into(),
            });
        }
        if let Transport::Stdio { command, .. } = transport {
            if !cfg.disabled.contains(name) && !command_exists(command, path_env, home) {
                findings.push(Finding {
                    severity: Severity::Critical,
                    tool: tool.id,
                    server: Some(name.clone()),
                    message: format!("command `{command}` was not found on this machine"),
                });
            }
        }

        let env_map = match transport {
            Transport::Stdio { env, .. } => env,
            Transport::Remote { headers, .. } => headers,
        };
        for (key, value) in env_map {
            for var_name in extract_env_var_refs(value) {
                if std::env::var_os(&var_name).is_none() {
                    findings.push(Finding {
                        severity: Severity::Warning,
                        tool: tool.id,
                        server: Some(name.clone()),
                        message: format!(
                            "env var `{var_name}` referenced in `{key}` is not set in this environment"
                        ),
                    });
                }
            }
        }

        if let Transport::Stdio { command, args, .. } = transport {
            if command == "npx" && !args.iter().any(|a| a == "-y" || a == "--yes") {
                findings.push(Finding {
                    severity: Severity::Warning,
                    tool: tool.id,
                    server: Some(name.clone()),
                    message: "npx without -y can hang waiting for interactive confirmation when the package is not cached".into(),
                });
            }
            if args.iter().any(|a| a.contains("@latest")) {
                findings.push(Finding {
                    severity: Severity::Warning,
                    tool: tool.id,
                    server: Some(name.clone()),
                    message: "@latest forces a registry round-trip on every launch — a common cause of -32001 timeouts; pin an exact version".into(),
                });
            }
        }
    }

    if cfg.servers.len() > CONTEXT_BLOAT_THRESHOLD {
        findings.push(Finding {
            severity: Severity::Info,
            tool: tool.id,
            server: None,
            message: format!(
                "{} servers configured — every connected server costs context window; consider pruning",
                cfg.servers.len()
            ),
        });
    }
}

/// Detect the same server name living in several tools with diverging or
/// identical configs.
fn drift_findings(tools: &[ToolLoad]) -> Vec<Finding> {
    let mut by_name: BTreeMap<&str, Vec<(&ToolId, &Transport)>> = BTreeMap::new();
    for tool in tools {
        if let ConfigState::Loaded(cfg) = &tool.state {
            for (name, transport) in &cfg.servers {
                by_name.entry(name).or_default().push((&tool.id, transport));
            }
        }
    }

    let mut findings = Vec::new();
    for (name, occurrences) in by_name {
        if occurrences.len() < 2 {
            continue;
        }
        let all_identical = occurrences.windows(2).all(|w| w[0].1 == w[1].1);
        if all_identical {
            let tools_list = occurrences
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            findings.push(Finding {
                severity: Severity::Info,
                tool: *occurrences[0].0,
                server: Some((*name).to_owned()),
                message: format!("identical duplicate in {tools_list}"),
            });
        } else {
            let mut sorted = occurrences.clone();
            sorted.sort_by_key(|(id, _)| *id);
            let tools_list = sorted
                .iter()
                .map(|(id, t)| format!("{} ({})", id.as_str(), t.summary()))
                .collect::<Vec<_>>()
                .join(" vs ");
            findings.push(Finding {
                severity: Severity::Warning,
                tool: *sorted[0].0,
                server: Some((*name).to_owned()),
                message: format!("config drift between tools: {tools_list}"),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::model::Servers;

    fn loaded(servers: Servers, problems: Vec<String>) -> ConfigState {
        loaded_with_disabled(servers, problems, BTreeSet::new())
    }

    fn loaded_with_disabled(
        servers: Servers,
        problems: Vec<String>,
        disabled: BTreeSet<String>,
    ) -> ConfigState {
        ConfigState::Loaded(Box::new(LoadedConfig {
            raw: crate::store::RawDoc::Json(serde_json::json!({})),
            servers,
            disabled,
            problems,
            editable: true,
        }))
    }

    #[test]
    fn parse_errors_are_critical() {
        let tools = vec![ToolLoad::new(
            ToolId::Cursor,
            Format::McpServers,
            ConfigState::ParseError("expected value".into()),
        )];
        let findings = diagnose(&tools, Path::new("/home/u"), "");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Critical);
    }

    #[test]
    fn npx_without_yes_is_warned() {
        let mut servers = Servers::new();
        servers.insert(
            "hangs".into(),
            Transport::Stdio {
                command: "npx".into(),
                args: vec!["@modelcontextprotocol/server-memory".into()],
                env: BTreeMap::new(),
            },
        );
        let tools = vec![ToolLoad::new(
            ToolId::Cursor,
            Format::McpServers,
            loaded(servers, vec![]),
        )];
        let findings = diagnose(&tools, Path::new("/home/u"), "");
        assert!(
            findings
                .iter()
                .any(|f| f.severity == Severity::Warning && f.message.contains("npx without -y"))
        );
    }

    #[test]
    fn npx_with_yes_flag_is_clean() {
        let mut servers = Servers::new();
        servers.insert(
            "fine".into(),
            Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "@modelcontextprotocol/server-memory".into()],
                env: BTreeMap::new(),
            },
        );
        let tools = vec![ToolLoad::new(
            ToolId::Cursor,
            Format::McpServers,
            loaded(servers, vec![]),
        )];
        let findings = diagnose(&tools, Path::new("/home/u"), "");
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("npx without -y"))
        );
    }

    #[test]
    fn latest_suffix_is_warned() {
        let mut servers = Servers::new();
        servers.insert(
            "slow".into(),
            Transport::Stdio {
                command: "npx".into(),
                args: vec![
                    "-y".into(),
                    "@modelcontextprotocol/server-memory@latest".into(),
                ],
                env: BTreeMap::new(),
            },
        );
        let tools = vec![ToolLoad::new(
            ToolId::Cursor,
            Format::McpServers,
            loaded(servers, vec![]),
        )];
        let findings = diagnose(&tools, Path::new("/home/u"), "");
        assert!(
            findings
                .iter()
                .any(|f| f.severity == Severity::Warning && f.message.contains("@latest forces"))
        );
    }

    #[test]
    fn missing_command_is_critical() {
        let mut servers = Servers::new();
        servers.insert(
            "broken".into(),
            Transport::Stdio {
                command: "/definitely/not/installed".into(),
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let tools = vec![ToolLoad::new(
            ToolId::Cursor,
            Format::McpServers,
            loaded(servers, vec![]),
        )];
        let findings = diagnose(&tools, Path::new("/home/u"), "");
        assert!(
            findings
                .iter()
                .any(|f| f.severity == Severity::Critical && f.message.contains("was not found"))
        );
    }

    #[test]
    fn disabled_servers_skip_dead_command_check() {
        let mut servers = Servers::new();
        servers.insert(
            "parked".into(),
            Transport::Stdio {
                command: "/definitely/not/installed".into(),
                args: vec![],
                env: BTreeMap::new(),
            },
        );
        let mut disabled = BTreeSet::new();
        disabled.insert("parked".to_owned());
        let tools = vec![ToolLoad::new(
            ToolId::Zed,
            Format::Zed,
            loaded_with_disabled(servers, vec![], disabled),
        )];
        let findings = diagnose(&tools, Path::new("/home/u"), "");
        assert!(
            findings.iter().all(|f| f.severity != Severity::Critical),
            "a parked server is intentional, not a finding: {findings:?}"
        );
    }

    #[test]
    fn drift_between_tools_is_a_warning() {
        let mut a = Servers::new();
        a.insert(
            "ctx".into(),
            Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "one".into()],
                env: BTreeMap::new(),
            },
        );
        let mut b = Servers::new();
        b.insert(
            "ctx".into(),
            Transport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "two".into()],
                env: BTreeMap::new(),
            },
        );
        let tools = vec![
            ToolLoad::new(ToolId::Cursor, Format::McpServers, loaded(a, vec![])),
            ToolLoad::new(ToolId::Zed, Format::Zed, loaded(b, vec![])),
        ];
        let findings = diagnose(&tools, Path::new("/home/u"), "/usr/bin");
        assert!(
            findings
                .iter()
                .any(|f| f.severity == Severity::Warning && f.message.contains("drift"))
        );
    }

    #[test]
    fn identical_duplicates_are_info() {
        let t = Transport::Stdio {
            command: "npx".into(),
            args: vec![],
            env: BTreeMap::new(),
        };
        let mut a = Servers::new();
        a.insert("same".into(), t.clone());
        let mut b = Servers::new();
        b.insert("same".into(), t);
        let tools = vec![
            ToolLoad::new(ToolId::Cursor, Format::McpServers, loaded(a, vec![])),
            ToolLoad::new(ToolId::Windsurf, Format::McpServers, loaded(b, vec![])),
        ];
        let findings = diagnose(&tools, Path::new("/home/u"), "/usr/bin");
        assert!(
            findings
                .iter()
                .any(|f| f.severity == Severity::Info && f.message.contains("identical"))
        );
    }

    #[test]
    fn extracts_env_var_references() {
        assert_eq!(
            extract_env_var_refs("${GITHUB_TOKEN}"),
            vec!["GITHUB_TOKEN"]
        );
        assert_eq!(extract_env_var_refs("${env:MY_KEY}"), vec!["MY_KEY"]);
        assert_eq!(
            extract_env_var_refs("Bearer ${env:API_KEY} extra ${OTHER}"),
            vec!["API_KEY", "OTHER"]
        );
        assert!(extract_env_var_refs("no templates here").is_empty());
        assert!(extract_env_var_refs("$NOT_BRACED").is_empty());
        assert!(extract_env_var_refs("${}").is_empty());
        assert!(extract_env_var_refs("${1abc}").is_empty());
    }

    #[test]
    fn command_lookup_respects_path() {
        assert!(!command_exists(
            "definitely-not-a-real-command-xyz",
            "",
            Path::new("/h")
        ));
        // Path lookup finds a file we create in a fake PATH directory.
        let dir = std::env::temp_dir().join(format!("mcpmedic-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("mcpmedic-fake-cmd");
        std::fs::write(&fake, "#!/bin/sh\n").unwrap();
        let path_env = dir.to_string_lossy().to_string();
        assert!(command_exists(
            "mcpmedic-fake-cmd",
            &path_env,
            Path::new("/h")
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
