//! Mutation commands: presets, add/rm, enable/disable, plus the shared
//! load/write/commit plumbing for editable configs.
//!
//! Split out of [`crate::commands`].

use std::collections::BTreeSet;
use std::path::Path;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::commands_ctx::{Ctx, commit, display_path, fail, print_json, resolve};
use crate::format::{self, Format};
use crate::format_fix;
use crate::format_toml;
use crate::model::{Servers, Transport};
use crate::registry::ToolSpec;
use crate::report;
use crate::store::{ConfigState, LoadedConfig, RawDoc};

/// Load a tool for mutation: it must parse, be editable, or (if missing) have
/// an existing parent directory so a fresh config can be created for it.
/// Returns the loaded config and whether the file needs creating on disk.
pub(crate) fn load_mutable(
    ctx: &Ctx,
    spec: &'static ToolSpec,
) -> Result<(LoadedConfig, bool), String> {
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

pub(crate) fn write_entry(
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

pub(crate) fn remove_entry(
    spec_format: Format,
    raw: &mut RawDoc,
    name: &str,
) -> Result<bool, String> {
    match raw {
        RawDoc::Json(doc) => Ok(format::remove_json_entry(spec_format, doc, name)),
        RawDoc::Toml(doc) => format_toml::remove_toml_entry(doc, name),
    }
}

/// Persist a mutated config (with automatic backup) unless this is a dry run.
pub(crate) fn parse_kv_pairs(
    items: &[String],
    flag: &str,
) -> Result<Vec<(String, String)>, String> {
    items
        .iter()
        .map(|item| match item.split_once('=') {
            Some((k, v)) if !k.is_empty() => Ok((k.to_owned(), v.to_owned())),
            _ => Err(format!("invalid {flag} value `{item}`, expected KEY=VALUE")),
        })
        .collect()
}

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

#[expect(clippy::too_many_arguments)]
pub(crate) fn cmd_add(
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
pub(crate) fn json_snippet(spec_format: Format, doc: &Value, name: &str) -> Option<String> {
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

pub(crate) fn cmd_rm(ctx: &Ctx, name: &str, from: &str, dry_run: bool) -> ExitCode {
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

pub(crate) fn cmd_set_enabled(
    ctx: &Ctx,
    name: &str,
    from: &str,
    enable: bool,
    dry_run: bool,
) -> ExitCode {
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
