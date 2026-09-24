//! Read commands: `scan`, `list`, `show`. Human tables plus `--json`
//! machine output, honoring `--quiet`.
//!
//! Split out of [`crate::commands`].

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::commands_ctx::{Ctx, display_path, fail, print_json, resolve};
use crate::model::Transport;
use crate::report;
use crate::store::ConfigState;

#[expect(clippy::too_many_lines)]
pub(crate) fn cmd_scan(ctx: &Ctx) -> ExitCode {
    if !ctx.json && !ctx.quiet {
        println!("{}", report::brand_header());
        if let Some(project) = &ctx.project {
            println!("  project: {}", project.display());
        }
        println!();
    }

    let mut configured = 0;
    let mut total_servers = 0;
    let mut with_findings = 0;
    let mut tools_json = Vec::new();

    for spec in ctx.specs() {
        let shown = display_path(&ctx.path(spec), &ctx.home);
        let mut entry = json!({
            "id": spec.id.as_str(),
            "display": spec.display,
            "path": ctx.path(spec).display().to_string(),
        });
        match ctx.load(spec) {
            ConfigState::Missing => {
                entry["state"] = json!("missing");
                if !ctx.json {
                    println!(
                        "  {} {:<14} {}",
                        report::glyph_none(),
                        spec.id.as_str(),
                        shown
                    );
                }
            }
            ConfigState::ParseError(msg) => {
                configured += 1;
                with_findings += 1;
                entry["state"] = json!("parse_error");
                entry["error"] = json!(msg);
                if !ctx.json {
                    println!(
                        "  {} {:<14} {}  ✗ parse error: {}",
                        report::glyph_crit(),
                        spec.id.as_str(),
                        shown,
                        report::truncate(&msg, 40)
                    );
                }
            }
            ConfigState::Loaded(cfg) => {
                configured += 1;
                total_servers += cfg.servers.len();
                entry["state"] = json!("loaded");
                entry["servers"] = json!(cfg.servers.len());
                entry["disabled"] = json!(cfg.disabled.len());
                entry["problems"] = json!(cfg.problems.len());
                if !cfg.problems.is_empty() {
                    with_findings += 1;
                }
                if !ctx.json {
                    if cfg.problems.is_empty() {
                        println!(
                            "  {} {:<14} {}  {} servers",
                            report::glyph_ok(),
                            spec.id.as_str(),
                            shown,
                            cfg.servers.len()
                        );
                    } else {
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
        tools_json.push(entry);
    }

    if ctx.json {
        print_json(&json!({
            "schema_version": 1,
            "generator": format!("mcpmedic {}", env!("CARGO_PKG_VERSION")),
            "project": ctx.project.as_ref().map(|p| p.display().to_string()),
            "tools": tools_json,
            "summary": {
                "configured": configured,
                "total_servers": total_servers,
                "with_findings": with_findings,
            }
        }));
        return ExitCode::SUCCESS;
    }

    println!();
    println!(
        "  {configured} tool(s) configured · {total_servers} servers total · {with_findings} with findings"
    );
    if (total_servers > 0 || with_findings > 0) && !ctx.quiet {
        println!(
            "  next: `mcpmedic doctor` for a health check, `mcpmedic list` to see every server"
        );
    }
    ExitCode::SUCCESS
}

#[expect(clippy::too_many_lines)]
pub(crate) fn cmd_list(ctx: &Ctx, tool: Option<&str>) -> ExitCode {
    let specs: Vec<&'static crate::registry::ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => ctx.specs(),
    };

    let mut any = false;
    let mut tools_json = Vec::new();
    for spec in specs {
        let mut entry = json!({
            "id": spec.id.as_str(),
            "display": spec.display,
            "path": ctx.path(spec).display().to_string(),
        });
        match ctx.load(spec) {
            ConfigState::Missing => {
                entry["state"] = json!("missing");
                if tool.is_some() && !ctx.json {
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
                entry["state"] = json!("parse_error");
                entry["error"] = json!(msg);
                if !ctx.json {
                    println!(
                        "  {} {} — parse error: {}",
                        report::glyph_crit(),
                        spec.display,
                        msg
                    );
                }
            }
            ConfigState::Loaded(cfg) => {
                any = true;
                entry["state"] = json!("loaded");
                entry["problems"] = json!(cfg.problems);
                let servers_json: Vec<Value> = cfg
                    .servers
                    .iter()
                    .map(|(name, transport)| {
                        json!({
                            "name": name,
                            "transport": transport.kind(),
                            "detail": transport.summary(),
                            "disabled": cfg.disabled.contains(name),
                        })
                    })
                    .collect();
                entry["servers"] = json!(servers_json);
                if !ctx.json {
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
                        let kind = if cfg.disabled.contains(name) {
                            format!("{} (off)", transport.kind())
                        } else {
                            transport.kind().to_owned()
                        };
                        rows.push(vec![
                            name.clone(),
                            kind,
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
        tools_json.push(entry);
    }

    if ctx.json {
        print_json(&json!({
            "schema_version": 1,
            "generator": format!("mcpmedic {}", env!("CARGO_PKG_VERSION")),
            "tools": tools_json,
        }));
        return ExitCode::SUCCESS;
    }

    if !any {
        println!("No MCP configs found. Install an AI tool, or add a server with:");
        println!(
            "  mcpmedic add context7 --to cursor --command npx --args -y @upstash/context7-mcp"
        );
    }
    ExitCode::SUCCESS
}

pub(crate) fn cmd_show(ctx: &Ctx, name: &str) -> ExitCode {
    let mut hits: Vec<(&'static crate::registry::ToolSpec, Transport, bool)> = Vec::new();
    for spec in ctx.specs() {
        if let ConfigState::Loaded(cfg) = ctx.load(spec) {
            if let Some(transport) = cfg.servers.get(name) {
                hits.push((spec, transport.clone(), cfg.disabled.contains(name)));
            }
        }
    }
    if hits.is_empty() {
        return fail(&format!("no tool configures a server named `{name}`"));
    }

    if ctx.json {
        let tools_json: Vec<Value> = hits
            .iter()
            .map(|(spec, transport, off)| {
                json!({
                    "id": spec.id.as_str(),
                    "display": spec.display,
                    "transport": transport.kind(),
                    "detail": transport.summary(),
                    "disabled": off,
                })
            })
            .collect();
        print_json(&json!({
            "schema_version": 1,
            "name": name,
            "tools": tools_json,
            "drift": hits.len() > 1 && !hits.windows(2).all(|w| w[0].1 == w[1].1),
        }));
        return ExitCode::SUCCESS;
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
    for (spec, transport, off) in &hits {
        let kind = if *off {
            format!("{} (off)", transport.kind())
        } else {
            transport.kind().to_owned()
        };
        rows.push(vec![
            spec.id.as_str().to_owned(),
            kind,
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
