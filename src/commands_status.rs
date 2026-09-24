//! Status commands: `init` one-shot onboarding and `summary` health line.
//!
//! Split out of [`crate::commands`].

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::commands_ctx::{Ctx, print_json};
use crate::commands_sync::cmd_sync_all;
use crate::doctor::{self, Severity, ToolLoad};
use crate::registry::ToolSpec;
use crate::report;
use crate::store::ConfigState;

pub(crate) fn cmd_init(ctx: &Ctx) -> ExitCode {
    let mut candidates: Vec<(&'static ToolSpec, usize)> = Vec::new();
    for spec in ctx.specs() {
        if let ConfigState::Loaded(cfg) = ctx.load(spec) {
            if !cfg.servers.is_empty() {
                candidates.push((spec, cfg.servers.len()));
            }
        }
    }

    if candidates.is_empty() {
        if ctx.json {
            print_json(&json!({
                "candidates": [],
                "next": "mcpmedic preset add minimal --to claude-code",
            }));
        } else {
            println!("{}", report::brand_header());
            if let Some(project) = &ctx.project {
                println!("  project: {}", project.display());
            }
            println!();
            println!("  No MCP servers found in any tool.");
            println!();
            println!("  Get started with a curated bundle:");
            println!("    mcpmedic preset add minimal --to claude-code");
            println!();
            println!("  Or add a single server:");
            println!(
                "    mcpmedic add claude-code demo -- npx -y @modelcontextprotocol/server-memory"
            );
            println!();
            println!("  Then run `mcpmedic init` again to sync it to every other installed tool.");
        }
        return ExitCode::SUCCESS;
    }

    candidates.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    let (source, count) = candidates[0];
    let source_id = source.id.as_str();

    if ctx.json {
        let list: Vec<Value> = candidates
            .iter()
            .map(|(spec, n)| json!({ "tool": spec.id.as_str(), "servers": n }))
            .collect();
        print_json(&json!({
            "candidates": list,
            "source": source_id,
            "servers": count,
            "next": format!("mcpmedic sync-all --from {source_id}"),
        }));
        return ExitCode::SUCCESS;
    }

    println!("{}", report::brand_header());
    if let Some(project) = &ctx.project {
        println!("  project: {}", project.display());
    }
    println!();
    println!("  MCP configs found:");
    for (spec, n) in &candidates {
        println!("    {:<14} {} server(s)", spec.id.as_str(), n);
    }
    println!();
    println!(
        "  Using {} as the source ({count} server(s), the most anywhere).",
        source.display
    );
    println!();

    let code = cmd_sync_all(ctx, source_id, &[], false, false);
    if code == ExitCode::SUCCESS {
        println!();
        println!("  Next: `mcpmedic doctor` verifies the merged setup is healthy.");
    }
    code
}

pub(crate) fn cmd_summary(ctx: &Ctx) -> ExitCode {
    let mut configured = 0;
    let mut total_servers = 0;
    let mut parked = 0;
    let loads: Vec<ToolLoad> = ctx
        .specs()
        .into_iter()
        .map(|spec| {
            let state = ctx.load(spec);
            if let ConfigState::Loaded(cfg) = &state {
                configured += 1;
                total_servers += cfg.servers.len();
                parked += cfg.disabled.len();
            }
            ToolLoad::new(spec.id, spec.format, state)
        })
        .collect();

    let path_env = std::env::var("PATH").unwrap_or_default();
    let findings = doctor::diagnose(&loads, &ctx.home, &path_env);
    let critical = findings
        .iter()
        .filter(|f| f.severity == Severity::Critical)
        .count();
    let warnings = findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();

    if ctx.json {
        print_json(&json!({
            "schema_version": 1,
            "generator": format!("mcpmedic {}", env!("CARGO_PKG_VERSION")),
            "tools": ctx.specs().len(),
            "configured": configured,
            "servers": total_servers,
            "parked": parked,
            "critical": critical,
            "warnings": warnings,
            "healthy": critical == 0,
        }));
        return if critical > 0 {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        };
    }

    println!(
        "mcpmedic: {} tools · {} configured · {} servers · {} parked · {} critical · {} warnings",
        ctx.specs().len(),
        configured,
        total_servers,
        parked,
        critical,
        warnings
    );
    if critical > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
