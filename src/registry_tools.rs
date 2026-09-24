//! The tool table: all 29 supported AI tools, their config paths,
//! dialects, and project-scoped paths.
//!
//! Split out of [`crate::registry`]. Paths stay derived from `home` so
//! everything remains testable against throwaway directories.

use std::path::{Path, PathBuf};

use crate::format::Format;
use crate::registry::{DisableFlag, ToolId, ToolSpec};

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
    ToolSpec {
        id: ToolId::LmStudio,
        display: "LM Studio",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: None,
        path: |home, _| home.join(".lmstudio").join("mcp.json"),
    },
    ToolSpec {
        id: ToolId::Continue,
        display: "Continue",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| {
            project
                .join(".continue")
                .join("mcpServers")
                .join("mcp.json")
        }),
        path: |home, _| home.join(".continue").join("mcpServers").join("mcp.json"),
    },
    ToolSpec {
        id: ToolId::Copilot,
        display: "GitHub Copilot CLI",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".github").join("mcp.json")),
        path: |home, _| home.join(".copilot").join("mcp-config.json"),
    },
    ToolSpec {
        id: ToolId::KimiCode,
        display: "Kimi Code",
        format: Format::McpServers,
        writable: true,
        disable: Some(DisableFlag {
            key: "enabled",
            off_when: false,
        }),
        project_path: Some(|project| project.join(".kimi-code").join("mcp.json")),
        path: |home, _| home.join(".kimi-code").join("mcp.json"),
    },
    ToolSpec {
        id: ToolId::QwenCode,
        display: "Qwen Code",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".qwen").join("settings.json")),
        path: |home, _| home.join(".qwen").join("settings.json"),
    },
    ToolSpec {
        id: ToolId::Auggie,
        display: "Auggie",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: None,
        path: |home, _| home.join(".augment").join("settings.json"),
    },
    ToolSpec {
        id: ToolId::FactoryDroid,
        display: "Factory Droid",
        format: Format::McpServers,
        writable: true,
        disable: Some(DisableFlag {
            key: "disabled",
            off_when: true,
        }),
        project_path: Some(|project| project.join(".factory").join("mcp.json")),
        path: |home, _| home.join(".factory").join("mcp.json"),
    },
    ToolSpec {
        id: ToolId::Amp,
        display: "Amp",
        format: Format::Amp,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".amp").join("settings.json")),
        path: |home, _| home.join(".config").join("amp").join("settings.json"),
    },
    ToolSpec {
        id: ToolId::Crush,
        display: "Crush",
        format: Format::Crush,
        writable: true,
        disable: Some(DisableFlag {
            key: "disabled",
            off_when: true,
        }),
        project_path: Some(|project| project.join("crush.json")),
        path: |home, _| home.join(".config").join("crush").join("crush.json"),
    },
    ToolSpec {
        id: ToolId::OpenHands,
        display: "OpenHands",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: None,
        path: |home, _| home.join(".openhands").join("mcp.json"),
    },
    ToolSpec {
        id: ToolId::Devin,
        display: "Devin CLI",
        format: Format::McpServers,
        writable: true,
        disable: None,
        project_path: Some(|project| project.join(".devin").join("config.json")),
        path: |home, _| home.join(".config").join("devin").join("config.json"),
    },
    ToolSpec {
        id: ToolId::OpenClaw,
        display: "OpenClaw",
        format: Format::OpenClaw,
        // Read-only like opencode: the config is JSON5 (comments and bare
        // keys are legal, and comments would be lost on a JSON rewrite) and
        // MCP servers sit under a nested `mcp.servers` block.
        writable: false,
        disable: Some(DisableFlag {
            key: "enabled",
            off_when: false,
        }),
        project_path: None,
        path: |home, _| home.join(".openclaw").join("openclaw.json"),
    },
    ToolSpec {
        id: ToolId::Goose,
        display: "Goose",
        format: Format::Yaml,
        // Read-only: config.yaml holds provider/model/preferences settings
        // that a YAML round-trip would mangle.
        writable: false,
        disable: Some(DisableFlag {
            key: "enabled",
            off_when: false,
        }),
        project_path: None,
        path: |home, _| home.join(".config").join("goose").join("config.yaml"),
    },
    ToolSpec {
        id: ToolId::ContinueYaml,
        display: "Continue (config.yaml)",
        format: Format::Yaml,
        // Read-only: config.yaml holds models/context/rules; the JSON
        // drop-in (~/.continue/mcpServers/mcp.json) stays the writable path.
        writable: false,
        disable: None,
        project_path: None,
        path: |home, _| home.join(".continue").join("config.yaml"),
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
