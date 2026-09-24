//! Preset commands: list curated bundles, install one into a tool.
//!
//! Split out of [`crate::commands_mutate`].

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::commands_ctx::{Ctx, commit, display_path, fail, print_json, resolve};
use crate::commands_mutate::{load_mutable, write_entry};
use crate::model::Transport;
use crate::report;

pub(crate) fn cmd_preset_list(ctx: &Ctx) -> ExitCode {
    if ctx.json {
        let list: Vec<Value> = crate::presets::PRESETS
            .iter()
            .map(|p| {
                json!({
                    "name": p.name,
                    "description": p.description,
                    "servers": p
                        .servers
                        .iter()
                        .map(|s| {
                            json!({"name": s.name, "command": s.command, "args": s.args})
                        })
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        print_json(&json!(list));
        return ExitCode::SUCCESS;
    }
    println!("{}", report::brand_header());
    println!();
    println!("  Curated bundles — `mcpmedic preset add <bundle> --to <tool>` installs one:");
    println!();
    for preset in crate::presets::PRESETS {
        println!("  {} — {}", preset.name, preset.description);
        for s in preset.servers {
            println!("      {}  {} {}", s.name, s.command, s.args.join(" "));
        }
        println!();
    }
    println!("  Then `mcpmedic sync-all --from <tool>` spreads it to every other installed tool.");
    ExitCode::SUCCESS
}

pub(crate) fn cmd_preset_add(
    ctx: &Ctx,
    name: &str,
    to: &str,
    force: bool,
    dry_run: bool,
) -> ExitCode {
    let Some(preset) = crate::presets::find(name) else {
        return fail(&format!(
            "unknown preset `{name}` — run `mcpmedic preset list` for the options"
        ));
    };
    let Ok(spec) = resolve(to) else {
        return fail(&format!("unknown tool `{to}`"));
    };
    let (mut cfg, _fresh) = match load_mutable(ctx, spec) {
        Ok(pair) => pair,
        Err(e) => return fail(&e),
    };

    let mut added = 0;
    for server in preset.servers {
        if !force && cfg.servers.contains_key(server.name) {
            println!(
                "  ~ `{}` already exists in {} — skipped (use --force to overwrite)",
                server.name, spec.display
            );
            continue;
        }
        let transport = Transport::Stdio {
            command: server.command.to_string(),
            args: server.args.iter().map(|a| (*a).to_string()).collect(),
            env: std::collections::BTreeMap::new(),
        };
        if let Err(e) = write_entry(spec, &mut cfg.raw, server.name, &transport) {
            return fail(&e);
        }
        let verb = if dry_run { "would land" } else { "added" };
        println!(
            "  + `{}` {verb} ({}) in {}",
            server.name,
            transport.kind(),
            spec.display
        );
        added += 1;
    }
    println!("      {}", display_path(&ctx.path(spec), &ctx.home));
    if added == 0 {
        println!("  nothing to do — every server in `{name}` is already configured");
        return ExitCode::SUCCESS;
    }
    match commit(ctx, spec, &cfg.raw, dry_run) {
        Ok(Some(backup)) => println!("      backup: {}", backup.display()),
        Ok(None) => {}
        Err(e) => return fail(&e),
    }
    if !dry_run {
        println!();
        println!(
            "  Next: `mcpmedic doctor --probe` verifies the bundle, then `mcpmedic sync-all --from {}` spreads it everywhere.",
            spec.id.as_str()
        );
    }
    ExitCode::SUCCESS
}
