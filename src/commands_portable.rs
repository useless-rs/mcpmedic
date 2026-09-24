//! Portable + snapshot commands: export/import, backup/restore.
//!
//! Split out of [`crate::commands`].

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::commands_ctx::{Ctx, commit, fail, resolve};
use crate::commands_mutate::{load_mutable, write_entry};
use crate::format::{self, Format};
use crate::model::Transport;
use crate::registry::{self, ToolId, ToolSpec};
use crate::report;
use crate::store::{self, ConfigState, backups_dir};

/// The portable interchange dialect for export/import: Claude Code's remote
/// shape (`type: "http"` + `url`), the most widely compatible spelling.
pub(crate) fn portable_entry(transport: &Transport) -> Value {
    format::build_json_entry(Format::McpServers, ToolId::ClaudeCode, transport)
}

pub(crate) fn cmd_export(ctx: &Ctx, out: Option<PathBuf>, tool: Option<&str>) -> ExitCode {
    let specs: Vec<&'static ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => ctx.specs(),
    };
    let mut tools = serde_json::Map::new();
    let mut server_count = 0;
    for spec in specs {
        if let ConfigState::Loaded(cfg) = ctx.load(spec) {
            if cfg.servers.is_empty() {
                continue;
            }
            let mut map = serde_json::Map::new();
            for (name, transport) in &cfg.servers {
                server_count += 1;
                map.insert(name.clone(), portable_entry(transport));
            }
            tools.insert(spec.id.as_str().to_owned(), Value::Object(map));
        }
    }

    let payload = json!({
        "version": 1,
        "generator": format!("mcpmedic {}", env!("CARGO_PKG_VERSION")),
        "tools": Value::Object(tools),
    });

    match out {
        Some(path) => {
            let body = format!(
                "{}\n",
                serde_json::to_string_pretty(&payload).unwrap_or_default()
            );
            if let Err(e) = store::write_atomic(&path, &body) {
                return fail(&format!("cannot write {}: {e}", path.display()));
            }
            println!(
                "  {} exported {server_count} server(s) → {}",
                report::glyph_ok(),
                path.display()
            );
        }
        None => println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        ),
    }
    ExitCode::SUCCESS
}

pub(crate) fn cmd_import(ctx: &Ctx, file: &Path, to: Option<&str>, dry_run: bool) -> ExitCode {
    let body = match std::fs::read_to_string(file) {
        Ok(b) => b,
        Err(e) => return fail(&format!("cannot read {}: {e}", file.display())),
    };
    let doc: Value = match serde_json::from_str(&body) {
        Ok(d) => d,
        Err(e) => return fail(&format!("invalid export file: {e}")),
    };

    // Two accepted shapes:
    //   {"tools": {"cursor": {...}}}  → each section merges into its own tool
    //   {"servers": {...}}            → merge into the tool given by --to
    let mut actions: Vec<(&'static ToolSpec, Vec<(String, Transport)>)> = Vec::new();

    if let Some(Value::Object(tools)) = doc.get("tools") {
        if to.is_some() {
            return fail(
                "--to is only valid when importing a file with a top-level `servers` section",
            );
        }
        for (tool_name, entries) in tools {
            let Some(spec) = registry::resolve_tool(tool_name) else {
                println!("  ? skipping unknown tool `{tool_name}`");
                continue;
            };
            if let Value::Object(map) = entries {
                let parsed = parse_export_map(map);
                if !parsed.is_empty() {
                    actions.push((spec, parsed));
                }
            }
        }
    } else if let Some(Value::Object(map)) = doc.get("servers") {
        let Some(target) = to else {
            return fail("this file has a top-level `servers` section — pass `--to <tool>`");
        };
        let Ok(spec) = resolve(target) else {
            return fail(&format!("unknown tool `{target}`"));
        };
        let parsed = parse_export_map(map);
        if !parsed.is_empty() {
            actions.push((spec, parsed));
        }
    } else {
        return fail(
            "file must contain either a `tools` or a `servers` object (as produced by `mcpmedic export`)",
        );
    }

    if actions.is_empty() {
        println!("  nothing to import");
        return ExitCode::SUCCESS;
    }

    for (spec, entries) in actions {
        let Ok((mut cfg, _fresh)) = load_mutable(ctx, spec) else {
            println!(
                "  {} cannot edit {} config — skipped",
                report::glyph_warn(),
                spec.display
            );
            continue;
        };
        let mut written = 0;
        for (name, transport) in entries {
            if cfg.servers.contains_key(&name) {
                println!("  = `{name}` already in {} — skipped", spec.display);
                continue;
            }
            if let Err(e) = write_entry(spec, &mut cfg.raw, &name, &transport) {
                println!("  {} `{name}`: {e}", report::glyph_crit());
                continue;
            }
            cfg.servers.insert(name.clone(), transport);
            written += 1;
            let verb = if dry_run { "would add" } else { "added" };
            println!("  + `{name}` {verb} ({})", spec.display);
        }
        if written == 0 || dry_run {
            continue;
        }
        match commit(ctx, spec, &cfg.raw, false) {
            Ok(Some(backup)) => println!("      backup: {}", backup.display()),
            Ok(None) => {}
            Err(e) => return fail(&e),
        }
    }
    ExitCode::SUCCESS
}

pub(crate) fn parse_export_map(map: &serde_json::Map<String, Value>) -> Vec<(String, Transport)> {
    let mut out = Vec::new();
    for (name, entry) in map {
        match format::parse_json_entry(entry) {
            Ok(transport) => out.push((name.clone(), transport)),
            Err(msg) => println!("  {} `{name}`: {msg}", report::glyph_warn()),
        }
    }
    out
}

pub(crate) fn cmd_restore(
    ctx: &Ctx,
    tool: Option<&str>,
    list: bool,
    latest: bool,
    dry_run: bool,
) -> ExitCode {
    let backups = store::list_backups(&ctx.home);

    if list || !latest {
        println!("{}", report::header("Available backups"));
        if backups.is_empty() {
            println!(
                "  no backups found in {}",
                store::backups_dir(&ctx.home).display()
            );
            println!("  backups are created automatically before every mcpmedic edit");
            return ExitCode::SUCCESS;
        }
        for entry in &backups {
            println!("  {} {}", entry.tool_id, entry.path.display());
        }
        println!();
        println!("  to restore: `mcpmedic restore --latest [--tool <id>]`");
        return ExitCode::SUCCESS;
    }

    let specs: Vec<&'static ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => ctx.specs(),
    };

    let mut restored = 0;
    for spec in specs {
        let Some(entry) = backups.iter().find(|b| b.tool_id == spec.id.as_str()) else {
            continue;
        };
        let path = ctx.path(spec);
        let contents = match std::fs::read_to_string(&entry.path) {
            Ok(c) => c,
            Err(e) => {
                println!(
                    "  {} cannot read backup {}: {e}",
                    report::glyph_crit(),
                    entry.path.display()
                );
                continue;
            }
        };
        if dry_run {
            restored += 1;
            println!(
                "  {} {} would restore from {}",
                report::glyph_ok(),
                spec.display,
                entry.path.display()
            );
            continue;
        }
        match store::persist(&path, &contents, spec.id.as_str(), &ctx.home) {
            Ok(_) => {
                restored += 1;
                println!(
                    "  {} {} restored from {}",
                    report::glyph_ok(),
                    spec.display,
                    entry.path.display()
                );
            }
            Err(e) => println!(
                "  {} {} restore failed: {e}",
                report::glyph_crit(),
                spec.display
            ),
        }
    }
    if restored == 0 {
        println!("  no backups found to restore");
    } else {
        println!("  restart the affected tool(s) for changes to take effect");
    }
    ExitCode::SUCCESS
}

pub(crate) fn cmd_backup(ctx: &Ctx, tool: Option<&str>) -> ExitCode {
    let specs: Vec<&'static ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => ctx.specs(),
    };

    let dir = backups_dir(&ctx.home);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(&format!("cannot create {}: {e}", dir.display()));
    }

    let mut count = 0;
    for spec in specs {
        let path = ctx.path(spec);
        if !path.exists() {
            continue;
        }
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("json");
        let target = dir.join(format!("{}-{millis}.{ext}", spec.id.as_str()));
        match std::fs::copy(&path, &target) {
            Ok(_) => {
                count += 1;
                println!(
                    "  {} {} → {}",
                    report::glyph_ok(),
                    spec.display,
                    target.display()
                );
            }
            Err(e) => println!("  {} {}: {e}", report::glyph_crit(), spec.display),
        }
    }
    if count == 0 {
        println!("  no configs found to back up");
    }
    ExitCode::SUCCESS
}
