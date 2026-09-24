//! Command-line interface definitions.

use std::path::PathBuf;

use clap::Parser;
use clap::builder::styling::{AnsiColor, Effects, Styles};

use crate::cli_cmd::Cmd;

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
