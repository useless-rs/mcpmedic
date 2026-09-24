//! `mcpmedic` — first aid for MCP configs.
//!
//! Every AI coding tool stores its MCP servers in its own config file, in its
//! own format, in its own corner of your home directory. `mcpmedic` reads them
//! all, normalizes them into one model, and lets you inspect, health-check,
//! diff and surgically edit them — with atomic writes and automatic backups.

mod audit;
mod cli;
mod commands;
mod diff;
mod doctor;
mod format;
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
    let cli = cli::Cli::parse();
    report::init_colors();
    commands::run(cli)
}
