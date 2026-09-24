//! Command implementations: the glue between the CLI and the engine.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::CommandFactory;
use serde_json::{Value, json};

use crate::audit;
use crate::cli::Cmd;
use crate::commands_ctx::{Ctx, commit, display_path, fail, print_json, resolve};
use crate::commands_doctor::{cmd_diff, cmd_doctor};
use crate::commands_read::{cmd_list, cmd_scan, cmd_show};
use crate::doctor::{self, Severity, ToolLoad};
use crate::format::{self, Format};
use crate::format_fix;
use crate::format_toml;
use crate::model::{Servers, Transport};
use crate::registry::{self, ToolId, ToolSpec};
use crate::report;
use crate::store::{self, ConfigState, LoadedConfig, RawDoc, backups_dir};
use crate::sync;

/// Entry point: dispatch a parsed CLI to a command.
pub(crate) fn run(cli: crate::cli::Cli) -> ExitCode {
    let ctx = Ctx::new(cli.project, cli.json, cli.quiet);
    let Some(cmd) = cli.cmd else {
        return cmd_scan(&ctx);
    };
    match cmd {
        Cmd::Scan => cmd_scan(&ctx),
        Cmd::Init => cmd_init(&ctx),
        Cmd::Preset { action } => match action {
            crate::cli::PresetAction::List => cmd_preset_list(&ctx),
            crate::cli::PresetAction::Add {
                name,
                to,
                force,
                dry_run,
            } => cmd_preset_add(&ctx, &name, &to, force, dry_run),
        },
        Cmd::List { tool } => cmd_list(&ctx, tool.as_deref()),
        Cmd::Show { name } => cmd_show(&ctx, &name),
        Cmd::Edit { tool, print } => cmd_edit(&ctx, &tool, print),
        Cmd::Doctor {
            tool,
            strict,
            fix,
            dry_run,
            probe,
            explain,
            probe_timeout,
        } => cmd_doctor(
            &ctx,
            tool.as_deref(),
            strict,
            fix,
            dry_run,
            probe,
            explain,
            probe_timeout,
        ),
        Cmd::Diff { a, b, exit_code } => cmd_diff(&ctx, &a, &b, exit_code),
        Cmd::Add {
            name,
            to,
            command,
            args,
            env,
            url,
            headers,
            dry_run,
        } => cmd_add(
            &ctx, &name, &to, command, args, &env, url, &headers, dry_run,
        ),
        Cmd::Rm {
            name,
            from,
            dry_run,
        } => cmd_rm(&ctx, &name, &from, dry_run),
        Cmd::Disable {
            name,
            from,
            dry_run,
        } => cmd_set_enabled(&ctx, &name, &from, false, dry_run),
        Cmd::Enable {
            name,
            from,
            dry_run,
        } => cmd_set_enabled(&ctx, &name, &from, true, dry_run),
        Cmd::Sync {
            from,
            to,
            names,
            force,
            dry_run,
        } => cmd_sync(&ctx, &from, &to, &names, force, dry_run),
        Cmd::SyncAll {
            from,
            names,
            force,
            dry_run,
        } => cmd_sync_all(&ctx, &from, &names, force, dry_run),
        Cmd::Export { out, tool } => cmd_export(&ctx, out, tool.as_deref()),
        Cmd::Import { file, to, dry_run } => cmd_import(&ctx, &file, to.as_deref(), dry_run),
        Cmd::Backup { tool } => cmd_backup(&ctx, tool.as_deref()),
        Cmd::Restore {
            tool,
            list,
            latest,
            dry_run,
        } => cmd_restore(&ctx, tool.as_deref(), list, latest, dry_run),
        Cmd::Audit { tool } => cmd_audit(&ctx, tool.as_deref()),
        Cmd::Summary => cmd_summary(&ctx),
        Cmd::Completions { shell } => cmd_completions(shell),
    }
}

fn cmd_completions(shell: clap_complete::Shell) -> ExitCode {
    let mut cmd = crate::cli::Cli::command();
    clap_complete::generate(shell, &mut cmd, "mcpmedic", &mut std::io::stdout());
    ExitCode::SUCCESS
}

fn cmd_edit(ctx: &Ctx, tool: &str, print: bool) -> ExitCode {
    let Ok(spec) = resolve(tool) else {
        return fail(&format!("unknown tool `{tool}`"));
    };
    if ctx.project.is_some() && spec.project_path.is_none() {
        return fail(&format!(
            "{} has no project-scoped config — --project does not apply to it",
            spec.display
        ));
    }
    let path = ctx.path(spec);
    if !path.exists() {
        return fail(&format!(
            "no config found for {} at {} — add one first with `mcpmedic add` or `mcpmedic preset add`",
            spec.display,
            display_path(&path, &ctx.home)
        ));
    }
    if ctx.json {
        print_json(&json!({
            "tool": spec.id.as_str(),
            "display": spec.display,
            "path": path.display().to_string(),
        }));
        return ExitCode::SUCCESS;
    }
    if print {
        println!("{}", path.display());
        return ExitCode::SUCCESS;
    }
    let editor = default_editor();
    match std::process::Command::new(&editor).arg(&path).status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => fail(&format!("editor `{editor}` exited with {status}")),
        Err(e) => fail(&format!("failed to launch editor `{editor}`: {e}")),
    }
}

fn default_editor() -> String {
    std::env::var("VISUAL")
        .ok()
        .or_else(|| std::env::var("EDITOR").ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        })
}

/// Load a tool for mutation: it must parse, be editable, or (if missing) have
/// an existing parent directory so a fresh config can be created for it.
/// Returns the loaded config and whether the file needs creating on disk.
fn load_mutable(ctx: &Ctx, spec: &'static ToolSpec) -> Result<(LoadedConfig, bool), String> {
    match ctx.load(spec) {
        ConfigState::Missing => {
            let parent = ctx
                .path(spec)
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            let may_create = if let Some(project) = &ctx.project {
                if spec.project_path.is_none() {
                    return Err(format!(
                        "{} has no project-scoped config — --project does not apply to it",
                        spec.display
                    ));
                }
                if !project.exists() {
                    return Err(format!(
                        "project directory {} does not exist",
                        project.display()
                    ));
                }
                let _ = std::fs::create_dir_all(&parent);
                true
            } else {
                parent.exists()
            };
            if may_create {
                let raw = if spec.format == Format::CodexToml {
                    RawDoc::Toml(toml_edit::DocumentMut::new())
                } else {
                    RawDoc::Json(json!({}))
                };
                Ok((
                    LoadedConfig {
                        raw,
                        servers: Servers::new(),
                        disabled: BTreeSet::new(),
                        problems: Vec::new(),
                        editable: true,
                    },
                    true,
                ))
            } else {
                Err(format!(
                    "no config found for {} and {} does not exist — is the tool installed?",
                    spec.display,
                    parent.display()
                ))
            }
        }
        ConfigState::ParseError(msg) => Err(format!(
            "{} config has a parse error ({msg}); mcpmedic refuses to edit broken files — run `mcpmedic doctor`",
            spec.display
        )),
        ConfigState::Loaded(boxed) => {
            let cfg = *boxed;
            if cfg.editable {
                Ok((cfg, false))
            } else {
                Err(format!(
                    "{} config is read-only for mcpmedic (JSONC comments or a not-yet-writable format)",
                    spec.display
                ))
            }
        }
    }
}

fn write_entry(
    spec: &ToolSpec,
    raw: &mut RawDoc,
    name: &str,
    transport: &Transport,
) -> Result<(), String> {
    match raw {
        RawDoc::Json(doc) => format::write_json_entry(spec.format, spec.id, doc, name, transport),
        RawDoc::Toml(doc) => format_toml::write_toml_entry(doc, name, transport),
    }
}

fn remove_entry(spec_format: Format, raw: &mut RawDoc, name: &str) -> Result<bool, String> {
    match raw {
        RawDoc::Json(doc) => Ok(format::remove_json_entry(spec_format, doc, name)),
        RawDoc::Toml(doc) => format_toml::remove_toml_entry(doc, name),
    }
}

/// Persist a mutated config (with automatic backup) unless this is a dry run.
fn parse_kv_pairs(items: &[String], flag: &str) -> Result<Vec<(String, String)>, String> {
    items
        .iter()
        .map(|item| match item.split_once('=') {
            Some((k, v)) if !k.is_empty() => Ok((k.to_owned(), v.to_owned())),
            _ => Err(format!("invalid {flag} value `{item}`, expected KEY=VALUE")),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// add / rm / sync
// ---------------------------------------------------------------------------

fn cmd_preset_list(ctx: &Ctx) -> ExitCode {
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

fn cmd_preset_add(ctx: &Ctx, name: &str, to: &str, force: bool, dry_run: bool) -> ExitCode {
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

#[expect(clippy::too_many_arguments)]
fn cmd_add(
    ctx: &Ctx,
    name: &str,
    to: &str,
    command: Option<String>,
    args: Vec<String>,
    env: &[String],
    url: Option<String>,
    headers: &[String],
    dry_run: bool,
) -> ExitCode {
    if name.trim().is_empty() {
        return fail("server name cannot be empty");
    }
    let Ok(spec) = resolve(to) else {
        return fail(&format!("unknown tool `{to}`"));
    };

    let transport = match (command, url) {
        (Some(_), Some(_)) => {
            return fail("pass either --command or --url, not both");
        }
        (None, Some(u)) => match parse_kv_pairs(headers, "--header") {
            Ok(pairs) => Transport::Remote {
                url: u,
                headers: pairs.into_iter().collect(),
            },
            Err(e) => return fail(&e),
        },
        (Some(c), None) => match parse_kv_pairs(env, "--env") {
            Ok(pairs) => Transport::Stdio {
                command: c,
                args,
                env: pairs.into_iter().collect(),
            },
            Err(e) => return fail(&e),
        },
        (None, None) => {
            return fail(
                "provide --command <executable> for a stdio server or --url <endpoint> for a remote one",
            );
        }
    };

    let (mut cfg, _fresh) = match load_mutable(ctx, spec) {
        Ok(pair) => pair,
        Err(e) => return fail(&e),
    };
    if cfg.servers.contains_key(name) {
        return fail(&format!(
            "`{name}` already exists in {} — remove it first: `mcpmedic rm {name} --from {}`",
            spec.display,
            spec.id.as_str()
        ));
    }

    if let Err(e) = write_entry(spec, &mut cfg.raw, name, &transport) {
        return fail(&e);
    }

    let verb = if dry_run { "would land" } else { "added" };
    println!(
        "  {} `{name}` {verb} ({}) in {}",
        report::glyph_ok(),
        transport.kind(),
        spec.display
    );
    println!("      {}", display_path(&ctx.path(spec), &ctx.home));

    let snippet = match &cfg.raw {
        RawDoc::Json(doc) => json_snippet(spec.format, doc, name),
        RawDoc::Toml(_) => Some(format!("[mcp_servers.{name}] {}", transport.summary())),
    };
    if let Some(snippet) = snippet {
        println!("      {}", report::truncate(&snippet, 74));
    }

    match commit(ctx, spec, &cfg.raw, dry_run) {
        Ok(Some(backup)) => println!("      backup: {}", backup.display()),
        Ok(None) => {}
        Err(e) => return fail(&e),
    }
    if !dry_run {
        println!(
            "      restart {} for the change to take effect",
            spec.display
        );
    }
    ExitCode::SUCCESS
}

/// Single-line JSON snippet of a freshly written entry, for confirmation.
fn json_snippet(spec_format: Format, doc: &Value, name: &str) -> Option<String> {
    let key = match spec_format {
        Format::McpServers | Format::CodexToml | Format::Opencode | Format::Yaml => "mcpServers",
        Format::Vscode => "servers",
        Format::Zed => "context_servers",
        Format::Amp => "amp.mcpServers",
        Format::Crush | Format::OpenClaw => "mcp",
    };
    let entry = doc.get(key)?.get(name)?;
    let pretty = serde_json::to_string_pretty(entry).unwrap_or_default();
    Some(pretty.lines().map(str::trim).collect::<Vec<_>>().join(" "))
}

fn cmd_rm(ctx: &Ctx, name: &str, from: &str, dry_run: bool) -> ExitCode {
    let Ok(spec) = resolve(from) else {
        return fail(&format!("unknown tool `{from}`"));
    };
    let (mut cfg, _fresh) = match load_mutable(ctx, spec) {
        Ok(pair) => pair,
        Err(e) => return fail(&e),
    };
    if !cfg.servers.contains_key(name) {
        return fail(&format!("`{name}` is not configured in {}", spec.display));
    }

    match remove_entry(spec.format, &mut cfg.raw, name) {
        Ok(true) => {}
        Ok(false) => {
            return fail(&format!(
                "`{name}` could not be removed from the {} config",
                spec.display
            ));
        }
        Err(e) => return fail(&e),
    }

    let verb = if dry_run { "would remove" } else { "removed" };
    println!(
        "  {} `{name}` {verb} from {}",
        report::glyph_ok(),
        spec.display
    );
    println!("      {}", display_path(&ctx.path(spec), &ctx.home));
    match commit(ctx, spec, &cfg.raw, dry_run) {
        Ok(Some(backup)) => println!("      backup: {}", backup.display()),
        Ok(None) => {}
        Err(e) => return fail(&e),
    }
    ExitCode::SUCCESS
}

fn cmd_set_enabled(ctx: &Ctx, name: &str, from: &str, enable: bool, dry_run: bool) -> ExitCode {
    let Ok(spec) = resolve(from) else {
        return fail(&format!("unknown tool `{from}`"));
    };
    let Some(flag) = spec.disable.as_ref() else {
        return fail(&format!(
            "{} has no documented per-server disable switch — use `mcpmedic rm {name} --from {}`",
            spec.display,
            spec.id.as_str()
        ));
    };
    let (mut cfg, _fresh) = match load_mutable(ctx, spec) {
        Ok(pair) => pair,
        Err(e) => return fail(&e),
    };
    if !cfg.servers.contains_key(name) {
        return fail(&format!("`{name}` is not configured in {}", spec.display));
    }

    let off = !enable;
    let changed = match &mut cfg.raw {
        RawDoc::Json(doc) => format_fix::set_json_disabled(flag, spec.format, doc, name, off),
        RawDoc::Toml(doc) => match format_toml::set_toml_disabled(flag, doc, name, off) {
            Ok(found) => found,
            Err(e) => return fail(&e),
        },
    };
    if !changed {
        return fail(&format!("`{name}` is not configured in {}", spec.display));
    }

    let (verb, state) = match (enable, dry_run) {
        (true, false) => ("resumed", "on"),
        (true, true) => ("would resume", "on"),
        (false, false) => ("parked", "off"),
        (false, true) => ("would park", "off"),
    };
    println!(
        "  {} `{name}` {verb} in {} ({state})",
        report::glyph_ok(),
        spec.display
    );
    println!("      {}", display_path(&ctx.path(spec), &ctx.home));
    match commit(ctx, spec, &cfg.raw, dry_run) {
        Ok(Some(backup)) => println!("      backup: {}", backup.display()),
        Ok(None) => {}
        Err(e) => return fail(&e),
    }
    ExitCode::SUCCESS
}

fn cmd_sync(
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

// ---------------------------------------------------------------------------
// export / import / backup
// ---------------------------------------------------------------------------

/// The portable interchange dialect for export/import: Claude Code's remote
/// shape (`type: "http"` + `url`), the most widely compatible spelling.
fn portable_entry(transport: &Transport) -> Value {
    format::build_json_entry(Format::McpServers, ToolId::ClaudeCode, transport)
}

fn cmd_export(ctx: &Ctx, out: Option<PathBuf>, tool: Option<&str>) -> ExitCode {
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

fn cmd_import(ctx: &Ctx, file: &Path, to: Option<&str>, dry_run: bool) -> ExitCode {
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

fn parse_export_map(map: &serde_json::Map<String, Value>) -> Vec<(String, Transport)> {
    let mut out = Vec::new();
    for (name, entry) in map {
        match format::parse_json_entry(entry) {
            Ok(transport) => out.push((name.clone(), transport)),
            Err(msg) => println!("  {} `{name}`: {msg}", report::glyph_warn()),
        }
    }
    out
}

fn cmd_audit(ctx: &Ctx, tool: Option<&str>) -> ExitCode {
    let specs: Vec<&'static ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => ctx.specs(),
    };

    let mut secrets = 0;
    let mut perms_warnings = 0;
    let mut findings_json = Vec::new();
    if !ctx.json {
        println!("{}", report::header("Security audit"));
    }
    for spec in specs {
        let ConfigState::Loaded(cfg) = ctx.load(spec) else {
            continue;
        };
        let path = ctx.path(spec);
        if let Some(warning) = audit::permissions_warning(&path) {
            perms_warnings += 1;
            findings_json.push(json!({
                "severity": "warning",
                "tool": spec.id.as_str(),
                "kind": "permissions",
                "message": format!("{} config is {warning}", spec.display),
            }));
            if !ctx.json {
                println!(
                    "  {} {} config is {warning}",
                    report::glyph_warn(),
                    spec.display
                );
            }
        }
        for (name, transport) in &cfg.servers {
            for finding in audit::scan_transport(name, transport) {
                secrets += 1;
                findings_json.push(json!({
                    "severity": "critical",
                    "tool": spec.id.as_str(),
                    "kind": "hardcoded_secret",
                    "server": finding.server,
                    "location": finding.location,
                    "message": format!("hardcoded {}", finding.label),
                }));
                if !ctx.json {
                    println!(
                        "  {} {} — server `{}` {}: hardcoded {}",
                        report::glyph_crit(),
                        spec.display,
                        finding.server,
                        finding.location,
                        finding.label
                    );
                }
            }
        }
    }

    if ctx.json {
        print_json(&json!({
            "schema_version": 1,
            "findings": findings_json,
            "summary": {
                "secrets": secrets,
                "permission_warnings": perms_warnings,
            }
        }));
        return if secrets > 0 {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        };
    }

    println!();
    println!("  {secrets} hardcoded secret(s) · {perms_warnings} permission warning(s)");
    if secrets > 0 {
        println!(
            "  rotate the exposed credentials; prefer env references (\"${{VAR}}\") over literals"
        );
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn cmd_restore(ctx: &Ctx, tool: Option<&str>, list: bool, latest: bool, dry_run: bool) -> ExitCode {
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

fn cmd_init(ctx: &Ctx) -> ExitCode {
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

fn cmd_sync_all(ctx: &Ctx, from: &str, names: &[String], force: bool, dry_run: bool) -> ExitCode {
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

fn cmd_summary(ctx: &Ctx) -> ExitCode {
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

fn cmd_backup(ctx: &Ctx, tool: Option<&str>) -> ExitCode {
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

#[cfg(test)]
mod tests {
    use crate::commands_doctor::fix_hint;

    #[test]
    fn fix_hints_match_known_categories() {
        assert!(fix_hint("config cannot be parsed: boom").is_some());
        assert!(fix_hint("command `npx` was not found on this machine").is_some());
        assert!(
            fix_hint("env var `X` referenced in `env` is not set in this environment").is_some()
        );
        assert!(fix_hint(
            "npx without -y can hang waiting for interactive confirmation when the package is not cached"
        )
        .is_some());
        assert!(fix_hint(
            "@latest forces a registry round-trip on every launch — a common cause of -32001 timeouts; pin an exact version"
        )
        .is_some());
        assert!(
            fix_hint("server `x`: VS Code requires a `type` field (`stdio` or `http`)").is_some()
        );
        assert!(fix_hint("identical duplicate in claude-code, cursor").is_some());
        assert!(fix_hint("config drift between tools: claude-code, cursor").is_some());
        assert!(
            fix_hint("unreachable: process exited with status 1 before answering MCP initialize")
                .is_some()
        );
        assert!(fix_hint("3 servers configured — consider pruning").is_some());
        assert!(fix_hint("probe: MCP handshake ok — `x` speaks protocol 2025-11-25").is_none());
        assert!(fix_hint("probe skipped: templated command").is_none());
    }
}
