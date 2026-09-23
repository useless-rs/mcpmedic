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

#[derive(Debug, Parser)]
#[command(
    name = "mcpmedic",
    version,
    about = "🚑 First aid for MCP configs: scan, doctor, diff and sync MCP servers across every AI tool you use.",
    propagate_version = true,
    styles = STYLES
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub cmd: Option<Cmd>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Cmd {
    /// Detect every installed AI tool and its MCP config (default command).
    Scan,

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
    Doctor {
        /// Only check this tool (id or display name).
        #[arg(long)]
        tool: Option<String>,

        /// Also fail on warnings, not just critical findings.
        #[arg(long)]
        strict: bool,
    },

    /// Compare the servers of two tools and show the drift.
    Diff {
        /// First tool (id or display name).
        a: String,
        /// Second tool (id or display name).
        b: String,
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

    /// Dump every discovered server into a portable JSON file (or stdout).
    Export {
        /// Write to this file instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
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
}
