//! `mcpmedic` — first aid for MCP configs.
//!
//! Every AI coding tool stores its MCP servers in its own config file, in its
//! own format, in its own corner of your home directory. `mcpmedic` reads them
//! all, normalizes them into one model, and lets you inspect, health-check,
//! diff and surgically edit them — with atomic writes and automatic backups.

mod audit;
mod cli;
mod commands;
mod commands_ctx;
mod commands_doctor;
mod commands_mutate;
mod commands_portable;
mod commands_read;
mod commands_sync;
mod diff;
mod doctor;
mod format;
mod format_fix;
mod format_jsonc;
mod format_toml;
mod format_yaml;
mod model;
mod presets;
mod probe;
mod registry;
mod report;
mod store;
mod sync;
mod yaml;

use std::process::ExitCode;

use clap::Parser;

fn main() -> ExitCode {
    silence_broken_pipe();
    let cli = cli::Cli::parse();
    report::init_colors();
    commands::run(cli)
}

/// Piping stdout to a closed reader (`mcpmedic list | head -1`) makes
/// `println!` panic with "failed printing to stdout: Broken pipe".
/// That is a successful truncated read, not a crash: exit quietly instead.
fn silence_broken_pipe() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info.to_string();
        if msg.contains("Broken pipe") || msg.contains("failed printing to stdout") {
            std::process::exit(0);
        }
        default(info);
    }));
}
