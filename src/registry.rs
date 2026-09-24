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
    LmStudio,
    Continue,
    Copilot,
    KimiCode,
    QwenCode,
    Auggie,
    FactoryDroid,
    Amp,
    Crush,
    OpenHands,
    Devin,
    OpenClaw,
    Goose,
    ContinueYaml,
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
            Self::LmStudio => "lm-studio",
            Self::Continue => "continue",
            Self::Copilot => "copilot",
            Self::KimiCode => "kimi-code",
            Self::QwenCode => "qwen-code",
            Self::Auggie => "auggie",
            Self::FactoryDroid => "factory-droid",
            Self::Amp => "amp",
            Self::Crush => "crush",
            Self::OpenHands => "openhands",
            Self::Devin => "devin",
            Self::OpenClaw => "openclaw",
            Self::Goose => "goose",
            Self::ContinueYaml => "continue-yaml",
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
/// Lives in [`crate::registry_tools`] next to the table.
pub(crate) use crate::registry_tools::registry;

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
        assert_eq!(registry().len(), 29);
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
        assert_eq!(
            by_id(ToolId::KimiCode).disable.as_ref().unwrap().key,
            "enabled"
        );
        assert!(!by_id(ToolId::KimiCode).disable.as_ref().unwrap().off_when);
        assert_eq!(
            by_id(ToolId::FactoryDroid).disable.as_ref().unwrap().key,
            "disabled"
        );
        assert!(
            by_id(ToolId::FactoryDroid)
                .disable
                .as_ref()
                .unwrap()
                .off_when
        );
        assert_eq!(
            by_id(ToolId::Crush).disable.as_ref().unwrap().key,
            "disabled"
        );
        assert!(by_id(ToolId::Crush).disable.as_ref().unwrap().off_when);
        assert_eq!(
            by_id(ToolId::OpenClaw).disable.as_ref().unwrap().key,
            "enabled"
        );
        assert!(!by_id(ToolId::OpenClaw).disable.as_ref().unwrap().off_when);
        assert_eq!(
            by_id(ToolId::Goose).disable.as_ref().unwrap().key,
            "enabled"
        );
        assert!(!by_id(ToolId::Goose).disable.as_ref().unwrap().off_when);
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
            ToolId::LmStudio,
            ToolId::Continue,
            ToolId::Copilot,
            ToolId::QwenCode,
            ToolId::Auggie,
            ToolId::Amp,
            ToolId::OpenHands,
            ToolId::Devin,
            ToolId::ContinueYaml,
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
        assert_eq!(
            project_of(ToolId::Copilot),
            PathBuf::from("/repo/.github/mcp.json")
        );
        assert_eq!(
            project_of(ToolId::KimiCode),
            PathBuf::from("/repo/.kimi-code/mcp.json")
        );
        assert_eq!(
            project_of(ToolId::QwenCode),
            PathBuf::from("/repo/.qwen/settings.json")
        );
        assert_eq!(
            project_of(ToolId::FactoryDroid),
            PathBuf::from("/repo/.factory/mcp.json")
        );
        assert_eq!(
            project_of(ToolId::Amp),
            PathBuf::from("/repo/.amp/settings.json")
        );
        assert_eq!(project_of(ToolId::Crush), PathBuf::from("/repo/crush.json"));
        assert_eq!(
            project_of(ToolId::Devin),
            PathBuf::from("/repo/.devin/config.json")
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
