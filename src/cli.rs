//! Command-line interface definitions.

use std::path::PathBuf;

use clap::builder::styling::{AnsiColor, Effects, Styles};
use clap::{Parser, Subcommand};

// mcpmedic help styling: green family for structure (headers/usage/literals),
// mirroring the brand's medical-green identity. anstream downgrades and
// disables these automatically for NO_COLOR and non-TTY output.
const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::BrightGreen.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::BrightBlack.on_default())
    .error(AnsiColor::Red.on_default().effects(Effects::BOLD))
    .valid(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .invalid(AnsiColor::Yellow.on_default().effects(Effects::BOLD));

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub(crate) enum ColorMode {
    /// Colors when on a terminal without `NO_COLOR`, off otherwise.
    #[default]
    Auto,
    /// Always emit ANSI colors, even when piped.
    Always,
    /// Never emit ANSI colors.
    Never,
}

#[derive(Debug, Parser)]
#[command(
    name = "mcpmedic",
    version,
    about = "🚑 First aid for MCP configs: scan, doctor, diff and sync MCP servers across every AI tool you use.",
    propagate_version = true,
    styles = STYLES
)]
pub(crate) struct Cli {
    /// Operate on project-scoped configs (`.mcp.json`, `.cursor/mcp.json`,
    /// `.vscode/mcp.json`, ...) rooted at this directory instead of the
    /// user-global files. Only tools that document a project scope
    /// participate.
    #[arg(long, global = true, value_name = "DIR")]
    pub project: Option<PathBuf>,

    /// Emit machine-readable JSON on stdout (scan, list, show, doctor,
    /// audit). Exit codes are unchanged; human decoration is suppressed.
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress decorative headers and hints; print only essential rows.
    /// Exit codes are unchanged. Combine with --json for fully scriptable output.
    #[arg(long, global = true)]
    pub quiet: bool,

    /// Control colored output. `auto` (default) disables colors when piped
    /// or when `NO_COLOR` is set; `always`/`never` force the behavior.
    #[arg(long, global = true, value_enum, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,

    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Cmd {
    /// Detect every installed AI tool and its MCP config (default command).
    Scan,

    /// One-shot onboarding: detect your configs, pick the richest as the
    /// source, and sync it to every other installed tool.
    /// With --json: print the detection report only, no files are touched.
    Init,

    /// List MCP servers, for every tool or one (`--tool`).
    List {
        /// Only show servers for this tool (id or display name).
        #[arg(long)]
        tool: Option<String>,
    },

    /// Show every place a given server name is configured.
    Show {
        /// Server name to look up across all tools.
        name: String,
    },

    /// Health-check all configs: broken JSON, dead commands, drift, bloat.
    /// Pass --probe to verify each server responds (real MCP initialize
    /// handshake and tools/list query for stdio servers, TCP connect
    /// for remotes).
    Doctor {
        /// Only check this tool (id or display name).
        #[arg(long)]
        tool: Option<String>,

        /// Also fail on warnings, not just critical findings.
        #[arg(long)]
        strict: bool,

        /// Apply safe automatic repairs (VS Code `type`, Zed legacy layout,
        /// remote `type` spellings, missing `npx -y`) before reporting.
        #[arg(long)]
        fix: bool,

        /// With `--fix`: show what would change without touching any file.
        #[arg(long, requires = "fix")]
        dry_run: bool,

        /// Probe each server: stdio servers get a real MCP initialize
        /// handshake plus a tools/list query — reporting the server's
        /// name, protocol version and exposed tool count; modern
        /// (2026-07-28-era) servers are identified via server/discover;
        /// remote endpoints get a TCP connect.
        #[arg(long)]
        probe: bool,

        /// After the findings, print how-to-fix guidance for each
        /// problem category found in this run.
        #[arg(long)]
        explain: bool,

        /// Per-exchange probe budget in milliseconds (default 3000).
        /// The discover and tools phases scale proportionally —
        /// raise this for servers with slow cold starts (npx/uvx
        /// package downloads). Only meaningful with --probe.
        #[arg(long, requires = "probe", value_name = "MS", default_value_t = 3000)]
        probe_timeout: u64,
    },

    /// Compare the servers of two tools and show the drift.
    Diff {
        /// First tool (id or display name).
        a: String,
        /// Second tool (id or display name).
        b: String,
        /// Exit 1 when drift is found, so CI can gate on tool parity.
        #[arg(long)]
        exit_code: bool,
    },

    /// Add a server to a tool's config.
    Add {
        /// Name of the server (e.g. `context7`).
        name: String,
        /// Target tool (id or display name).
        #[arg(long)]
        to: String,
        /// Executable for a local stdio server (e.g. `npx`).
        #[arg(long, conflicts_with = "url")]
        command: Option<String>,
        /// Everything after `--` is passed as arguments to `--command`.
        #[arg(last = true, requires = "command", allow_hyphen_values = true)]
        args: Vec<String>,
        /// Environment variables, repeatable (e.g. `--env KEY=value`).
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// URL for a remote HTTP server (mutually exclusive with `--command`).
        #[arg(long, conflicts_with = "command")]
        url: Option<String>,
        /// HTTP headers for a remote server, repeatable.
        #[arg(long = "header", value_name = "KEY=VALUE")]
        headers: Vec<String>,
        /// Show what would be written without touching any file.
        #[arg(long)]
        dry_run: bool,
    },

    /// Curated zero-config server bundles: list them or add one.
    Preset {
        #[command(subcommand)]
        action: PresetAction,
    },

    /// Remove a server from a tool's config.
    Rm {
        /// Name of the server to remove.
        name: String,
        /// Source tool (id or display name).
        #[arg(long)]
        from: String,
        /// Show what would be removed without touching any file.
        #[arg(long)]
        dry_run: bool,
    },

    /// Park a server without removing its config, where the tool documents a
    /// per-server disable switch.
    Disable {
        /// Name of the server to park.
        name: String,
        /// Tool that owns the config (id or display name).
        #[arg(long)]
        from: String,
        /// Show the change without touching any file.
        #[arg(long)]
        dry_run: bool,
    },

    /// Resume a server parked with `mcpmedic disable`.
    Enable {
        /// Name of the server to resume.
        name: String,
        /// Tool that owns the config (id or display name).
        #[arg(long)]
        from: String,
        /// Show the change without touching any file.
        #[arg(long)]
        dry_run: bool,
    },

    /// Copy servers from one tool to another (additive merge, never deletes).
    Sync {
        /// Source tool (id or display name).
        #[arg(long)]
        from: String,
        /// Target tool (id or display name).
        #[arg(long)]
        to: String,
        /// Only sync these server names (comma-separated).
        #[arg(long, value_delimiter = ',')]
        names: Vec<String>,
        /// Also overwrite servers that exist in the target with a different config.
        #[arg(long)]
        force: bool,
        /// Show the plan without touching any file.
        #[arg(long)]
        dry_run: bool,
    },

    /// Sync from one tool into every other detected tool (one-to-many).
    SyncAll {
        /// Source tool (id or display name).
        #[arg(long)]
        from: String,
        /// Only sync these server names (comma-separated).
        #[arg(long, value_delimiter = ',')]
        names: Vec<String>,
        /// Also overwrite servers that exist in targets with different configs.
        #[arg(long)]
        force: bool,
        /// Show the plan without touching any file.
        #[arg(long)]
        dry_run: bool,
    },

    /// Dump every discovered server into a portable JSON file (or stdout).
    Export {
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Only export this tool (id or display name).
        #[arg(long)]
        tool: Option<String>,
    },

    /// Merge servers from an exported file back into your tools.
    Import {
        /// File produced by `mcpmedic export` (or a hand-written one).
        file: PathBuf,
        /// Target tool when importing a top-level `servers` file.
        #[arg(long)]
        to: Option<String>,
        /// Show what would be written without touching any file.
        #[arg(long)]
        dry_run: bool,
    },

    /// Security audit: hardcoded secrets in env/header values and config
    /// file permissions.
    Audit {
        /// Only audit this tool (id or display name).
        #[arg(long)]
        tool: Option<String>,
    },

    /// Restore configs from automatic backups (list or --latest to restore).
    Restore {
        /// Only restore this tool (id or display name).
        #[arg(long)]
        tool: Option<String>,
        /// List available backups without restoring.
        #[arg(long)]
        list: bool,
        /// Restore the most recent backup for each tool (or one with --tool).
        #[arg(long)]
        latest: bool,
        /// Show what would be restored without touching any file.
        #[arg(long, requires = "latest")]
        dry_run: bool,
    },

    /// One-line health overview (tools, servers, findings) for shell prompts and CI gates.
    Summary,

    /// Generate shell completions to stdout (bash, zsh, fish, elvish, powershell).
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Manually back up configs (also happens automatically before every edit).
    Backup {
        /// Only back up this tool (id or display name).
        #[arg(long)]
        tool: Option<String>,
    },

    /// Open a tool's config file in your editor.
    Edit {
        /// Tool whose config to open (id or display name).
        tool: String,
        /// Print the resolved config path instead of opening it.
        #[arg(long)]
        print: bool,
    },
}

#[derive(Debug, clap::Subcommand)]
pub(crate) enum PresetAction {
    /// Show the available curated bundles and their servers.
    List,

    /// Add every server in a bundle to one tool.
    Add {
        /// Bundle name (see `mcpmedic preset list`).
        name: String,
        /// Target tool (id or display name).
        #[arg(long)]
        to: String,
        /// Overwrite an existing server that has the same name.
        #[arg(long)]
        force: bool,
        /// Show what would change without touching any file.
        #[arg(long)]
        dry_run: bool,
    },
}
