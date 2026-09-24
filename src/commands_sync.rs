//! Sync commands: one-to-one `sync` and one-to-many `sync-all`.
//! Additive merges only — targets are never deleted. Drift handling,
//! dry-runs, and per-target reporting live here.
//!
//! Split out of [`crate::commands`].

use std::process::ExitCode;

use crate::commands_ctx::{Ctx, commit, display_path, fail, resolve};
use crate::commands_mutate::{load_mutable, write_entry};
use crate::registry::ToolSpec;
use crate::report;
use crate::store::ConfigState;
use crate::sync;

pub(crate) fn cmd_sync(
    ctx: &Ctx,
    from: &str,
    to: &str,
    names: &[String],
    force: bool,
    dry_run: bool,
) -> ExitCode {
    let (Ok(spec_from), Ok(spec_to)) = (resolve(from), resolve(to)) else {
        return fail(&format!(
            "unknown tool in `mcpmedic sync --from {from} --to {to}`"
        ));
    };
    if spec_from.id == spec_to.id {
        return fail("source and target are the same tool");
    }

    let source_servers = match ctx.load(spec_from) {
        ConfigState::Missing => {
            return fail(&format!(
                "no config found for {} ({})",
                spec_from.display,
                display_path(&ctx.path(spec_from), &ctx.home)
            ));
        }
        ConfigState::ParseError(msg) => {
            return fail(&format!(
                "{} config has a parse error: {msg}",
                spec_from.display
            ));
        }
        ConfigState::Loaded(cfg) => cfg.servers,
    };

    let (mut target, _fresh) = match load_mutable(ctx, spec_to) {
        Ok(pair) => pair,
        Err(e) => return fail(&e),
    };

    let plan = sync::plan(&source_servers, &target.servers, names, force);

    println!(
        "{}",
        report::header(&format!("sync {} → {}", spec_from.display, spec_to.display))
    );
    for (name, transport) in &plan.to_add {
        let verb = if dry_run { "would add" } else { "added" };
        println!("  + {name} ({verb}, {})", transport.kind());
        println!(
            "      {} {}",
            transport.kind(),
            report::truncate(&transport.summary(), 66)
        );
    }
    for name in &plan.drifted {
        if force {
            println!("  ~ {name} (overwritten with --force)");
        } else {
            println!("  ~ {name} drifted — skipped (use --force to overwrite)");
        }
    }
    if !plan.identical.is_empty() {
        println!(
            "  = {} already identical: {}",
            plan.identical.len(),
            plan.identical.join(", ")
        );
    }
    if !plan.unknown.is_empty() {
        println!("  ? not found in source: {}", plan.unknown.join(", "));
    }
    if plan.to_add.is_empty() {
        println!("  ✓ nothing to do — target is already in sync");
    }

    if plan.to_add.is_empty() || dry_run {
        return ExitCode::SUCCESS;
    }

    for (name, transport) in &plan.to_add {
        if let Err(e) = write_entry(spec_to, &mut target.raw, name, transport) {
            return fail(&e);
        }
    }
    match commit(ctx, spec_to, &target.raw, false) {
        Ok(Some(backup)) => println!("  backup: {}", backup.display()),
        Ok(None) => {}
        Err(e) => return fail(&e),
    }
    println!("  restart {} for changes to take effect", spec_to.display);
    ExitCode::SUCCESS
}

pub(crate) fn cmd_sync_all(
    ctx: &Ctx,
    from: &str,
    names: &[String],
    force: bool,
    dry_run: bool,
) -> ExitCode {
    let Ok(spec_from) = resolve(from) else {
        return fail(&format!("unknown tool `{from}`"));
    };

    let source_servers = match ctx.load(spec_from) {
        ConfigState::Missing => {
            return fail(&format!(
                "no config found for {} ({})",
                spec_from.display,
                display_path(&ctx.path(spec_from), &ctx.home)
            ));
        }
        ConfigState::ParseError(msg) => {
            return fail(&format!(
                "{} config has a parse error: {msg}",
                spec_from.display
            ));
        }
        ConfigState::Loaded(cfg) => cfg.servers,
    };

    println!(
        "{}",
        report::header(&format!(
            "sync-all: {} ({} server(s)) → every other detected tool",
            spec_from.display,
            source_servers.len()
        ))
    );

    let targets: Vec<&'static ToolSpec> = ctx
        .specs()
        .into_iter()
        .filter(|s| s.id != spec_from.id)
        .collect();

    let mut synced = 0;
    for target_spec in targets {
        let (mut target, _) = match load_mutable(ctx, target_spec) {
            Ok(pair) => pair,
            Err(e) => {
                println!("  ~ {} skipped: {e}", target_spec.display);
                continue;
            }
        };
        let plan = sync::plan(&source_servers, &target.servers, names, force);
        if plan.to_add.is_empty() && plan.drifted.is_empty() && plan.unknown.is_empty() {
            continue;
        }
        for name in &plan.drifted {
            if force {
                println!(
                    "  ~ {name} (overwritten with --force) in {}",
                    target_spec.display
                );
            } else {
                println!(
                    "  ~ {name} drifted in {} — skipped (use --force to overwrite)",
                    target_spec.display
                );
            }
        }
        if !plan.unknown.is_empty() {
            println!("  ? not found in source: {}", plan.unknown.join(", "));
        }
        if plan.to_add.is_empty() {
            continue;
        }
        if dry_run {
            for (name, transport) in &plan.to_add {
                println!(
                    "  + {name} (would add, {}) in {}",
                    transport.kind(),
                    target_spec.display
                );
            }
            synced += 1;
            continue;
        }
        for (name, transport) in &plan.to_add {
            if let Err(e) = write_entry(target_spec, &mut target.raw, name, transport) {
                return fail(&e);
            }
        }
        match commit(ctx, target_spec, &target.raw, false) {
            Ok(_) => {
                synced += 1;
                println!(
                    "  {} {} server(s) → {}",
                    report::glyph_ok(),
                    plan.to_add.len(),
                    target_spec.display
                );
            }
            Err(e) => return fail(&e),
        }
    }

    if synced == 0 {
        println!("  nothing to sync — all detected tools are already covered");
    } else if !dry_run {
        println!("  restart the affected tool(s) for changes to take effect");
    }
    ExitCode::SUCCESS
}
