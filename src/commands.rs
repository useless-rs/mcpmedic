//! Command implementations: the glue between the CLI and the engine.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::CommandFactory;
use serde_json::{Value, json};

use crate::cli::Cmd;
use crate::diff;
use crate::doctor::{self, Severity, ToolLoad};
use crate::format::{self, Format};
use crate::model::{Servers, Transport};
use crate::registry::{self, EnvOverrides, ToolSpec};
use crate::report;
use crate::store::{self, ConfigState, LoadedConfig, RawDoc, backups_dir};
use crate::sync;

/// Everything commands need to operate, resolved once.
struct Ctx {
    home: PathBuf,
    env: EnvOverrides,
}

impl Ctx {
    fn new() -> Self {
        Self {
            home: home_dir(),
            env: EnvOverrides::from_env(),
        }
    }

    fn path(&self, spec: &ToolSpec) -> PathBuf {
        (spec.path)(&self.home, &self.env)
    }

    fn load(&self, spec: &'static ToolSpec) -> ConfigState {
        store::load(spec, &self.home, &self.env)
    }
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|h| !h.is_empty())
                .map(PathBuf::from)
        })
        .or_else(dirs::home_dir)
        .unwrap_or_default()
}

/// Entry point: dispatch a parsed CLI to a command.
pub(crate) fn run(cli: crate::cli::Cli) -> ExitCode {
    let Some(cmd) = cli.cmd else {
        return cmd_scan();
    };
    let ctx = Ctx::new();
    match cmd {
        Cmd::Scan => cmd_scan(),
        Cmd::List { tool } => cmd_list(&ctx, tool.as_deref()),
        Cmd::Show { name } => cmd_show(&ctx, &name),
        Cmd::Doctor { tool, strict } => cmd_doctor(&ctx, tool.as_deref(), strict),
        Cmd::Diff { a, b } => cmd_diff(&ctx, &a, &b),
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
        Cmd::Sync {
            from,
            to,
            names,
            force,
            dry_run,
        } => cmd_sync(&ctx, &from, &to, &names, force, dry_run),
        Cmd::Export { out } => cmd_export(&ctx, out),
        Cmd::Import { file, to, dry_run } => cmd_import(&ctx, &file, to.as_deref(), dry_run),
        Cmd::Backup { tool } => cmd_backup(&ctx, tool.as_deref()),
        Cmd::Completions { shell } => cmd_completions(shell),
    }
}

fn cmd_completions(shell: clap_complete::Shell) -> ExitCode {
    let mut cmd = crate::cli::Cli::command();
    clap_complete::generate(shell, &mut cmd, "mcpmedic", &mut std::io::stdout());
    ExitCode::SUCCESS
}

fn fail(message: &str) -> ExitCode {
    eprintln!("{}", report::error_line(message));
    ExitCode::from(2)
}

fn resolve(name: &str) -> Result<&'static ToolSpec, String> {
    registry::resolve_tool(name)
        .ok_or_else(|| format!("unknown tool `{name}` — run `mcpmedic scan` to list tools"))
}

/// Render a path relative to the home directory as `~/...` when possible.
fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
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
            if parent.exists() {
                let raw = if spec.format == Format::CodexToml {
                    RawDoc::Toml(toml_edit::DocumentMut::new())
                } else {
                    RawDoc::Json(json!({}))
                };
                Ok((
                    LoadedConfig {
                        raw,
                        servers: Servers::new(),
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
    spec_format: Format,
    raw: &mut RawDoc,
    name: &str,
    transport: &Transport,
) -> Result<(), String> {
    match raw {
        RawDoc::Json(doc) => format::write_json_entry(spec_format, doc, name, transport),
        RawDoc::Toml(doc) => format::write_toml_entry(doc, name, transport),
    }
}

fn remove_entry(spec_format: Format, raw: &mut RawDoc, name: &str) -> Result<bool, String> {
    match raw {
        RawDoc::Json(doc) => Ok(format::remove_json_entry(spec_format, doc, name)),
        RawDoc::Toml(doc) => format::remove_toml_entry(doc, name),
    }
}

/// Persist a mutated config (with automatic backup) unless this is a dry run.
fn commit(
    ctx: &Ctx,
    spec: &ToolSpec,
    raw: &RawDoc,
    dry_run: bool,
) -> Result<Option<PathBuf>, String> {
    if dry_run {
        return Ok(None);
    }
    let path = ctx.path(spec);
    store::persist(&path, &raw.serialize(), spec.id.as_str(), &ctx.home)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))
}

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
// scan / list / show
// ---------------------------------------------------------------------------

fn cmd_scan() -> ExitCode {
    let ctx = Ctx::new();
    println!("{}", report::brand_header());
    println!();

    let mut configured = 0;
    let mut total_servers = 0;
    let mut with_findings = 0;

    for spec in registry::registry() {
        let shown = display_path(&ctx.path(spec), &ctx.home);
        match ctx.load(spec) {
            ConfigState::Missing => {
                println!(
                    "  {} {:<14} {}",
                    report::glyph_none(),
                    spec.id.as_str(),
                    shown
                );
            }
            ConfigState::ParseError(msg) => {
                configured += 1;
                with_findings += 1;
                println!(
                    "  {} {:<14} {}  ✗ parse error: {}",
                    report::glyph_crit(),
                    spec.id.as_str(),
                    shown,
                    report::truncate(&msg, 40)
                );
            }
            ConfigState::Loaded(cfg) => {
                configured += 1;
                total_servers += cfg.servers.len();
                if cfg.problems.is_empty() {
                    println!(
                        "  {} {:<14} {}  {} servers",
                        report::glyph_ok(),
                        spec.id.as_str(),
                        shown,
                        cfg.servers.len()
                    );
                } else {
                    with_findings += 1;
                    println!(
                        "  {} {:<14} {}  ✗ {} unparseable entries",
                        report::glyph_ok(),
                        spec.id.as_str(),
                        shown,
                        cfg.problems.len()
                    );
                }
            }
        }
    }

    println!();
    println!(
        "  {configured} tool(s) configured · {total_servers} servers total · {with_findings} with findings"
    );
    if total_servers > 0 || with_findings > 0 {
        println!(
            "  next: `mcpmedic doctor` for a health check, `mcpmedic list` to see every server"
        );
    }
    ExitCode::SUCCESS
}

fn cmd_list(ctx: &Ctx, tool: Option<&str>) -> ExitCode {
    let specs: Vec<&'static ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => registry::registry().iter().collect(),
    };

    let mut any = false;
    for spec in specs {
        match ctx.load(spec) {
            ConfigState::Missing => {
                if tool.is_some() {
                    println!(
                        "  {} {} — no config found at {}",
                        report::glyph_none(),
                        spec.display,
                        display_path(&ctx.path(spec), &ctx.home)
                    );
                }
            }
            ConfigState::ParseError(msg) => {
                any = true;
                println!(
                    "  {} {} — parse error: {}",
                    report::glyph_crit(),
                    spec.display,
                    msg
                );
            }
            ConfigState::Loaded(cfg) => {
                any = true;
                println!(
                    "\n{} — {} ({})",
                    report::header(spec.display),
                    display_path(&ctx.path(spec), &ctx.home),
                    cfg.servers.len()
                );
                if cfg.servers.is_empty() && cfg.problems.is_empty() {
                    println!("  (no servers configured)");
                    continue;
                }
                let mut rows = vec![vec![
                    "NAME".to_owned(),
                    "TRANSPORT".to_owned(),
                    "COMMAND / URL".to_owned(),
                ]];
                for (name, transport) in &cfg.servers {
                    rows.push(vec![
                        name.clone(),
                        transport.kind().to_owned(),
                        report::truncate(&transport.summary(), 58),
                    ]);
                }
                print!("{}", report::render_table(&rows));
                if !cfg.problems.is_empty() {
                    println!(
                        "  {} {} unparseable entries — run `mcpmedic doctor`",
                        report::glyph_warn(),
                        cfg.problems.len()
                    );
                }
            }
        }
    }
    if !any {
        println!("No MCP configs found. Install an AI tool, or add a server with:");
        println!(
            "  mcpmedic add context7 --to cursor --command npx --args -y @upstash/context7-mcp"
        );
    }
    ExitCode::SUCCESS
}

fn cmd_show(ctx: &Ctx, name: &str) -> ExitCode {
    let mut hits: Vec<(&'static ToolSpec, Transport)> = Vec::new();
    for spec in registry::registry() {
        if let ConfigState::Loaded(cfg) = ctx.load(spec) {
            if let Some(transport) = cfg.servers.get(name) {
                hits.push((spec, transport.clone()));
            }
        }
    }
    if hits.is_empty() {
        return fail(&format!("no tool configures a server named `{name}`"));
    }

    println!(
        "{}",
        report::header(&format!("`{name}` is configured in {} tool(s)", hits.len()))
    );
    let mut rows = vec![vec![
        "TOOL".to_owned(),
        "TRANSPORT".to_owned(),
        "CONFIG".to_owned(),
    ]];
    for (spec, transport) in &hits {
        rows.push(vec![
            spec.id.as_str().to_owned(),
            transport.kind().to_owned(),
            report::truncate(&transport.summary(), 58),
        ]);
    }
    print!("{}", report::render_table(&rows));

    let all_identical = hits.windows(2).all(|w| w[0].1 == w[1].1);
    if hits.len() > 1 && !all_identical {
        println!(
            "\n  {} configs differ between tools — this is drift; see `mcpmedic doctor`",
            report::glyph_warn()
        );
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// doctor / diff
// ---------------------------------------------------------------------------

fn cmd_doctor(ctx: &Ctx, tool: Option<&str>, strict: bool) -> ExitCode {
    let specs: Vec<&'static ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => registry::registry().iter().collect(),
    };

    let loads: Vec<ToolLoad> = specs
        .into_iter()
        .map(|spec| ToolLoad::new(spec.id, spec.format, ctx.load(spec)))
        .collect();

    let path_env = std::env::var("PATH").unwrap_or_default();
    let findings = doctor::diagnose(&loads, &ctx.home, &path_env);

    if findings.is_empty() {
        println!(
            "  {} checked {} config(s) — everything looks healthy",
            report::glyph_ok(),
            loads
                .iter()
                .filter(|t| !matches!(t.state, ConfigState::Missing))
                .count()
        );
        return ExitCode::SUCCESS;
    }

    let mut by_severity = [0_usize; 3];
    println!("{}", report::header("Health report"));
    for spec in registry::registry() {
        let tool_findings: Vec<&doctor::Finding> =
            findings.iter().filter(|f| f.tool == spec.id).collect();
        if tool_findings.is_empty() {
            continue;
        }
        println!(
            "\n  {} — {}",
            report::header(spec.display),
            display_path(&ctx.path(spec), &ctx.home)
        );
        for f in tool_findings {
            let (glyph, index) = match f.severity {
                Severity::Critical => (report::glyph_crit(), 0),
                Severity::Warning => (report::glyph_warn(), 1),
                Severity::Info => (report::glyph_info(), 2),
            };
            by_severity[index] += 1;
            let subject = f
                .server
                .as_deref()
                .map(|s| format!("server `{s}`: "))
                .unwrap_or_default();
            println!(
                "  {glyph} {:<8} {subject}{}",
                severity_label(f.severity),
                f.message
            );
        }
    }

    println!();
    println!(
        "  {} critical · {} warning(s) · {} info",
        by_severity[0], by_severity[1], by_severity[2]
    );
    if by_severity[0] > 0 || (strict && by_severity[1] > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

fn cmd_diff(ctx: &Ctx, a: &str, b: &str) -> ExitCode {
    let (Ok(spec_a), Ok(spec_b)) = (resolve(a), resolve(b)) else {
        return fail(&format!("unknown tool in `mcpmedic diff {a} {b}`"));
    };

    let load_servers = |spec: &'static ToolSpec| -> Result<Servers, String> {
        match ctx.load(spec) {
            ConfigState::Missing => Err(format!(
                "no config found for {} ({})",
                spec.display,
                display_path(&ctx.path(spec), &ctx.home)
            )),
            ConfigState::ParseError(msg) => {
                Err(format!("{} config has a parse error: {msg}", spec.display))
            }
            ConfigState::Loaded(cfg) => Ok(cfg.servers),
        }
    };
    let servers_a = match load_servers(spec_a) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };
    let servers_b = match load_servers(spec_b) {
        Ok(s) => s,
        Err(e) => return fail(&e),
    };

    let result = diff::diff(&servers_a, &servers_b);
    println!(
        "{}",
        report::header(&format!(
            "{} ({}) → {} ({})",
            spec_a.display,
            servers_a.len(),
            spec_b.display,
            servers_b.len()
        ))
    );

    if !result.only_a.is_empty() {
        println!("  ~ only in {}:", spec_a.id.as_str());
        println!("      {}", result.only_a.join(", "));
    }
    if !result.only_b.is_empty() {
        println!("  ~ only in {}:", spec_b.id.as_str());
        println!("      {}", result.only_b.join(", "));
    }
    if !result.different.is_empty() {
        println!("  ✗ same name, different config:");
        for (name, reason) in &result.different {
            println!("      {name} — {reason}");
        }
    }
    println!("  = identical servers: {}", result.identical);
    if result.only_a.is_empty() && result.only_b.is_empty() && result.different.is_empty() {
        println!("  ✓ no drift — both tools are in sync");
    }
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// add / rm / sync
// ---------------------------------------------------------------------------

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

    if let Err(e) = write_entry(spec.format, &mut cfg.raw, name, &transport) {
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
        Format::McpServers | Format::CodexToml | Format::Opencode => "mcpServers",
        Format::Vscode => "servers",
        Format::Zed => "context_servers",
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
        if let Err(e) = write_entry(spec_to.format, &mut target.raw, name, transport) {
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

fn cmd_export(ctx: &Ctx, out: Option<PathBuf>) -> ExitCode {
    let mut tools = serde_json::Map::new();
    let mut server_count = 0;
    for spec in registry::registry() {
        if let ConfigState::Loaded(cfg) = ctx.load(spec) {
            if cfg.servers.is_empty() {
                continue;
            }
            let mut map = serde_json::Map::new();
            for (name, transport) in &cfg.servers {
                server_count += 1;
                map.insert(
                    name.clone(),
                    format::build_json_entry(Format::McpServers, transport),
                );
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
            if let Err(e) = std::fs::write(&path, body) {
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
            if let Err(e) = write_entry(spec.format, &mut cfg.raw, &name, &transport) {
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

fn cmd_backup(ctx: &Ctx, tool: Option<&str>) -> ExitCode {
    let specs: Vec<&'static ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => registry::registry().iter().collect(),
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
