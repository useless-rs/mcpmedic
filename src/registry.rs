//! The registry of supported AI tools: where each keeps its MCP config and in
//! which format. All paths are derived from a `home` directory so the whole
//! crate is testable against throwaway home directories.

use std::path::{Path, PathBuf};

use crate::format::Format;

/// Supported AI tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum ToolId {
    ClaudeCode,
    ClaudeDesktop,
    Cursor,
    Windsurf,
    Vscode,
    Zed,
    GeminiCli,
    Codex,
    Cline,
    RooCode,
    Opencode,
    Warp,
    Kiro,
    Trae,
    Antigravity,
}

impl ToolId {
    /// Kebab-case identifier accepted on the command line.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Cursor => "cursor",
            Self::Windsurf => "windsurf",
            Self::Vscode => "vscode",
            Self::Zed => "zed",
            Self::GeminiCli => "gemini-cli",
            Self::Codex => "codex",
            Self::Cline => "cline",
            Self::RooCode => "roo-code",
            Self::Opencode => "opencode",
            Self::Warp => "warp",
            Self::Kiro => "kiro",
            Self::Trae => "trae",
            Self::Antigravity => "antigravity",
        }
    }
}

/// Environment-driven location overrides, resolved once at startup so tests
/// can inject their own.
#[derive(Clone, Debug, Default)]
pub(crate) struct EnvOverrides {
    /// `CODEX_HOME` — Codex keeps `config.toml` in here instead of `~/.codex`.
    pub codex_home: Option<PathBuf>,
    /// `CLAUDE_CONFIG_DIR` — Claude Code reads `.claude.json` from here.
    pub claude_config_dir: Option<PathBuf>,
}

impl EnvOverrides {
    /// Collect overrides from the real process environment.
    pub(crate) fn from_env() -> Self {
        Self {
            codex_home: std::env::var_os("CODEX_HOME").map(PathBuf::from),
            claude_config_dir: std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
        }
    }
}

/// How a tool parks a server without removing it: the config field name and
/// the boolean value that means "off". Only set where the tool documents or
/// observably persists the flag (Codex `enabled`, Cline and Roo Code
/// `disabled`, Zed `enabled`).
pub(crate) struct DisableFlag {
    /// Config key on the server entry.
    pub key: &'static str,
    /// The value of `key` that parks the server.
    pub off_when: bool,
}

/// Static description of one tool's MCP config file.
pub(crate) struct ToolSpec {
    /// Canonical id.
    pub id: ToolId,
    /// Human-friendly name.
    pub display: &'static str,
    /// Serialization format of the config file.
    pub format: Format,
    /// Whether `mcpmedic` may edit this file (opencode is read-only for now).
    pub writable: bool,
    /// The tool's per-server disable switch, where documented. `enable` and
    /// `disable` refuse to run for tools without one.
    pub disable: Option<DisableFlag>,
    /// The project-scoped config path, where the tool documents one. In
    /// `--project` mode only these tools participate.
    pub project_path: Option<fn(&Path) -> PathBuf>,
    /// Resolve the config path relative to a home directory.
    pub path: fn(&Path, &EnvOverrides) -> PathBuf,
}

/// The tools `mcpmedic` knows about, in a stable display order.
pub(crate) fn registry() -> &'static [ToolSpec] {
    REGISTRY
}

static REGISTRY: &[ToolSpec] = &[
    ToolSpec {
        id: ToolId::ClaudeCode,
        display: "Claude Code",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".mcp.json")),
        path: |home, env| {
            env.claude_config_dir
                .clone()
                .map_or_else(|| home.join(".claude.json"), |d| d.join(".claude.json"))
        },
    },
    ToolSpec {
        id: ToolId::ClaudeDesktop,
        display: "Claude Desktop",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: None,
        path: |home, _| {
            if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Claude/claude_desktop_config.json")
            } else if cfg!(windows) {
                home.join("AppData/Roaming/Claude/claude_desktop_config.json")
            } else {
                home.join(".config/Claude/claude_desktop_config.json")
            }
        },
    },
    ToolSpec {
        id: ToolId::Cursor,
        display: "Cursor",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".cursor").join("mcp.json")),
        path: |home, _| home.join(".cursor").join("mcp.json"),
    },
    ToolSpec {
        id: ToolId::Windsurf,
        display: "Windsurf",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: None,
        path: |home, _| {
            if cfg!(windows) {
                home.join("AppData/Roaming/Codeium/windsurf/mcp_config.json")
            } else {
                home.join(".codeium/windsurf/mcp_config.json")
            }
        },
    },
    ToolSpec {
        id: ToolId::Vscode,
        display: "VS Code",
        format: Format::Vscode,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".vscode").join("mcp.json")),
        path: |home, _| home.join(".vscode").join("mcp.json"),
    },
    ToolSpec {
        id: ToolId::Zed,
        display: "Zed",
        format: Format::Zed,
        writable: true,
        disable: Some(DisableFlag {
            key: "enabled",
            off_when: false,
        }),
        project_path: None,
        path: |home, _| {
            if cfg!(windows) {
                home.join("AppData/Roaming/Zed/settings.json")
            } else {
                home.join(".config/zed/settings.json")
            }
        },
    },
    ToolSpec {
        id: ToolId::GeminiCli,
        display: "Gemini CLI",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".gemini").join("settings.json")),
        path: |home, _| home.join(".gemini").join("settings.json"),
    },
    ToolSpec {
        id: ToolId::Codex,
        display: "Codex CLI",
        format: Format::CodexToml,
        writable: true,
        disable: Some(DisableFlag {
            key: "enabled",
            off_when: false,
        }),
        project_path: Some(|project| project.join(".codex").join("config.toml")),
        path: |home, env| {
            env.codex_home
                .clone()
                .unwrap_or_else(|| home.join(".codex"))
                .join("config.toml")
        },
    },
    ToolSpec {
        id: ToolId::Cline,
        display: "Cline",
        format: Format::McpServers,
        writable: true,
        disable: Some(DisableFlag {
            key: "disabled",
            off_when: true,
        }),
        project_path: None,
        path: |home, _| {
            vscode_global_storage(home)
                .join("saoudrizwan.claude-dev/settings/cline_mcp_settings.json")
        },
    },
    ToolSpec {
        id: ToolId::RooCode,
        display: "Roo Code",
        format: Format::McpServers,
        writable: true,
        disable: Some(DisableFlag {
            key: "disabled",
            off_when: true,
        }),
        project_path: None,
        path: |home, _| {
            vscode_global_storage(home)
                .join("rooveterinaryinc.roo-cline/settings/mcp_settings.json")
        },
    },
    ToolSpec {
        id: ToolId::Opencode,
        display: "opencode",
        format: Format::Opencode,
        // Read-only for now: opencode configs are JSONC (comments are lost on
        // a JSON rewrite) and the v2 layout (`mcp.servers`) is still moving.
        writable: false,
        disable: None,
        project_path: Some(|project| project.join("opencode.json")),
        path: |home, _| {
            let json = home.join(".config/opencode/opencode.json");
            let jsonc = home.join(".config/opencode/opencode.jsonc");
            if jsonc.exists() && !json.exists() {
                jsonc
            } else {
                json
            }
        },
    },
    ToolSpec {
        id: ToolId::Warp,
        display: "Warp",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".warp").join(".mcp.json")),
        path: |home, _| home.join(".warp").join(".mcp.json"),
    },
    ToolSpec {
        id: ToolId::Kiro,
        display: "Kiro",
        format: Format::McpServers,
        writable: true,
        disable: Some(DisableFlag {
            key: "disabled",
            off_when: true,
        }),
        project_path: Some(|project| project.join(".kiro").join("settings").join("mcp.json")),
        path: |home, _| home.join(".kiro").join("settings").join("mcp.json"),
    },
    ToolSpec {
        id: ToolId::Trae,
        display: "TRAE",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: None,
        path: |home, _| {
            if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Trae/User/mcp.json")
            } else if cfg!(windows) {
                home.join("AppData/Roaming/Trae/User/mcp.json")
            } else {
                home.join(".config/Trae/User/mcp.json")
            }
        },
    },
    ToolSpec {
        id: ToolId::Antigravity,
        display: "Antigravity",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".agents").join("mcp_config.json")),
        path: |home, _| home.join(".gemini").join("config").join("mcp_config.json"),
    },
];

/// VS Code user-data directory that extension global storage lives under.
fn vscode_global_storage(home: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/Application Support/Code")
    } else if cfg!(windows) {
        home.join("AppData/Roaming/Code")
    } else {
        home.join(".config/Code")
    }
}

/// Resolve a user-supplied tool name (id or display, case-insensitive).
pub(crate) fn resolve_tool(input: &str) -> Option<&'static ToolSpec> {
    let needle = input.trim().to_ascii_lowercase();
    registry().iter().find(|spec| {
        spec.id.as_str() == needle
            || spec.display.to_ascii_lowercase() == needle
            || spec.id.as_str().replace('-', "") == needle.replace(['-', ' '], "")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_ids_and_display_names() {
        assert_eq!(resolve_tool("cursor").unwrap().id, ToolId::Cursor);
        assert_eq!(resolve_tool("Cursor").unwrap().id, ToolId::Cursor);
        assert_eq!(resolve_tool("claude-code").unwrap().id, ToolId::ClaudeCode);
        assert_eq!(resolve_tool("Claude Code").unwrap().id, ToolId::ClaudeCode);
        assert_eq!(resolve_tool("VS Code").unwrap().id, ToolId::Vscode);
        assert_eq!(resolve_tool("roo-code").unwrap().id, ToolId::RooCode);
        assert!(resolve_tool("nope").is_none());
    }

    #[test]
    fn registry_is_complete() {
        assert_eq!(registry().len(), 15);
        for spec in registry() {
            assert!(!spec.display.is_empty());
        }
    }

    #[test]
    fn disable_flags_are_set_only_where_documented() {
        let by_id = |id: ToolId| registry().iter().find(|s| s.id == id).unwrap();
        assert_eq!(
            by_id(ToolId::Codex).disable.as_ref().unwrap().key,
            "enabled"
        );
        assert!(!by_id(ToolId::Codex).disable.as_ref().unwrap().off_when);
        assert_eq!(
            by_id(ToolId::Cline).disable.as_ref().unwrap().key,
            "disabled"
        );
        assert!(by_id(ToolId::Cline).disable.as_ref().unwrap().off_when);
        assert_eq!(
            by_id(ToolId::RooCode).disable.as_ref().unwrap().key,
            "disabled"
        );
        assert_eq!(by_id(ToolId::Zed).disable.as_ref().unwrap().key, "enabled");
        assert_eq!(
            by_id(ToolId::Kiro).disable.as_ref().unwrap().key,
            "disabled"
        );
        assert!(by_id(ToolId::Kiro).disable.as_ref().unwrap().off_when);
        for id in [
            ToolId::Cursor,
            ToolId::ClaudeCode,
            ToolId::ClaudeDesktop,
            ToolId::Windsurf,
            ToolId::Vscode,
            ToolId::GeminiCli,
            ToolId::Opencode,
            ToolId::Warp,
            ToolId::Trae,
            ToolId::Antigravity,
        ] {
            assert!(
                by_id(id).disable.is_none(),
                "{id:?} must not claim an undocumented flag"
            );
        }
    }

    #[test]
    fn project_paths_are_set_only_where_documented() {
        let by_id = |id: ToolId| registry().iter().find(|s| s.id == id).unwrap();
        let project = Path::new("/repo");
        let project_of = |id: ToolId| (by_id(id).project_path.unwrap())(project);
        assert_eq!(
            project_of(ToolId::ClaudeCode),
            PathBuf::from("/repo/.mcp.json")
        );
        assert_eq!(
            project_of(ToolId::Cursor),
            PathBuf::from("/repo/.cursor/mcp.json")
        );
        assert_eq!(
            project_of(ToolId::Vscode),
            PathBuf::from("/repo/.vscode/mcp.json")
        );
        assert_eq!(
            project_of(ToolId::GeminiCli),
            PathBuf::from("/repo/.gemini/settings.json")
        );
        assert_eq!(
            project_of(ToolId::Codex),
            PathBuf::from("/repo/.codex/config.toml")
        );
        assert_eq!(
            project_of(ToolId::Opencode),
            PathBuf::from("/repo/opencode.json")
        );
        assert_eq!(
            project_of(ToolId::Warp),
            PathBuf::from("/repo/.warp/.mcp.json")
        );
        assert_eq!(
            project_of(ToolId::Kiro),
            PathBuf::from("/repo/.kiro/settings/mcp.json")
        );
        assert_eq!(
            project_of(ToolId::Antigravity),
            PathBuf::from("/repo/.agents/mcp_config.json")
        );
        for id in [
            ToolId::ClaudeDesktop,
            ToolId::Windsurf,
            ToolId::Zed,
            ToolId::Cline,
            ToolId::RooCode,
            ToolId::Trae,
        ] {
            assert!(
                by_id(id).project_path.is_none(),
                "{id:?} must not claim a project scope without first-party docs"
            );
        }
    }

    #[test]
    fn codex_home_override_wins() {
        let env = EnvOverrides {
            codex_home: Some(PathBuf::from("/custom/codex")),
            claude_config_dir: None,
        };
        let spec = registry().iter().find(|s| s.id == ToolId::Codex).unwrap();
        let p = (spec.path)(Path::new("/home/u"), &env);
        assert_eq!(p, PathBuf::from("/custom/codex/config.toml"));

        let default_env = EnvOverrides::default();
        let p = (spec.path)(Path::new("/home/u"), &default_env);
        assert_eq!(p, PathBuf::from("/home/u/.codex/config.toml"));
    }
}
