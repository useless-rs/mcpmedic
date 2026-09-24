//! Command implementations: the glue between the CLI and the engine.

use std::process::ExitCode;

use clap::CommandFactory;
use serde_json::{Value, json};

use crate::audit;
use crate::cli::Cmd;
use crate::commands_ctx::{Ctx, display_path, fail, print_json, resolve};
use crate::commands_doctor::{cmd_diff, cmd_doctor};
use crate::commands_mutate::{cmd_add, cmd_preset_add, cmd_preset_list, cmd_rm, cmd_set_enabled};
use crate::commands_portable::{cmd_backup, cmd_export, cmd_import, cmd_restore};
use crate::commands_read::{cmd_list, cmd_scan, cmd_show};
use crate::commands_sync::{cmd_sync, cmd_sync_all};
use crate::doctor::{self, Severity, ToolLoad};
use crate::registry::ToolSpec;
use crate::report;
use crate::store::ConfigState;

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

// ---------------------------------------------------------------------------
// export / import / backup
// ---------------------------------------------------------------------------

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
