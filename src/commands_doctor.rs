//! Doctor + diff commands: health checks with `--fix`/`--probe`/
//! `--explain`, and cross-tool drift reports.
//!
//! Split out of [`crate::commands`].

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::commands_ctx::{Ctx, commit, display_path, fail, print_json, resolve};
use crate::commands_probe::probe_findings;
use crate::doctor::{self, Severity, ToolLoad};
use crate::format_fix;
use crate::format_toml;
use crate::registry::ToolSpec;
use crate::report;
use crate::store::{ConfigState, RawDoc};

pub(crate) fn fix_hint(message: &str) -> Option<&'static str> {
    if message.contains("config cannot be parsed") {
        return Some(
            "fix the JSON/TOML syntax by hand — `mcpmedic show <tool>` prints the exact path; mcpmedic refuses to edit broken files",
        );
    }
    if message.contains("was not found on this machine") {
        return Some(
            "install the binary, use an absolute path, or remove the server: `mcpmedic rm <name> --from <tool>`",
        );
    }
    if message.contains("is not set in this environment") {
        return Some(
            "export the variable in the shell profile your AI tool inherits, or set the value directly",
        );
    }
    if message.contains("npx without -y") {
        return Some(
            "`mcpmedic doctor --fix` inserts the missing -y automatically (preview with --dry-run)",
        );
    }
    if message.contains("@latest forces a registry round-trip") {
        return Some(
            "pin an exact version (e.g. pkg@2.1.0) — rewrite the entry with `mcpmedic rm` + `mcpmedic add`",
        );
    }
    if message.contains("requires a `type` field") || message.contains("no `type`") {
        return Some("`mcpmedic doctor --fix` adds the required type field automatically");
    }
    if message.contains("is not accepted by") {
        return Some(
            "`mcpmedic doctor --fix` corrects the type spelling to the tool's dialect automatically",
        );
    }
    if message.contains("entry is not a JSON object") || message.contains("entry has neither") {
        return Some("every server needs `command` (stdio) or a URL field (remote)");
    }
    if message.contains("identical duplicate") {
        return Some(
            "identical duplicates are harmless; remove extras with `mcpmedic rm <name> --from <tool>`",
        );
    }
    if message.contains("config drift between tools") {
        return Some(
            "`mcpmedic diff <tool-a> <tool-b>` shows the drift; `mcpmedic sync` propagates one way",
        );
    }
    if message.contains("consider pruning") {
        return Some(
            "park servers you rarely use: `mcpmedic disable <name> --from <tool>` — parked servers stay in the file but are skipped",
        );
    }
    if message.contains("unreachable: process exited") {
        return Some(
            "the stderr tail names the real cause — fix it and re-run `mcpmedic doctor --probe --tool <tool>` to verify",
        );
    }
    if message.contains("unreachable: failed to start") {
        return Some(
            "the command could not even be spawned — check the name, path and permissions",
        );
    }
    if message.contains("unreachable: connect failed") {
        return Some("check the URL, the port and whether the endpoint is up");
    }
    if message.contains("no MCP initialize response within") {
        return Some(
            "slow startup is common with npx — run your IDE once to warm the npm cache, then re-probe",
        );
    }
    if message.contains("server name is empty") {
        return Some("rename the server — most tools refuse empty keys");
    }
    None
}

#[expect(clippy::too_many_lines)]
#[expect(clippy::fn_params_excessive_bools)]
#[expect(clippy::too_many_arguments)]
pub(crate) fn cmd_doctor(
    ctx: &Ctx,
    tool: Option<&str>,
    strict: bool,
    fix: bool,
    dry_run: bool,
    probe: bool,
    explain: bool,
    probe_timeout: u64,
) -> ExitCode {
    let specs: Vec<&'static ToolSpec> = match tool {
        Some(name) => match resolve(name) {
            Ok(spec) => vec![spec],
            Err(e) => return fail(&e),
        },
        None => ctx.specs(),
    };

    let mut loads: Vec<ToolLoad> = specs
        .iter()
        .copied()
        .map(|spec| ToolLoad::new(spec.id, spec.format, ctx.load(spec)))
        .collect();

    let path_env = std::env::var("PATH").unwrap_or_default();
    let mut findings = doctor::diagnose(&loads, &ctx.home, &path_env);

    if fix {
        match apply_repairs(ctx, &specs, &mut loads, dry_run) {
            Ok(printed) if printed => println!(),
            Ok(_) => {}
            Err(e) => return fail(&e),
        }
        findings = doctor::diagnose(&loads, &ctx.home, &path_env);
    }

    if probe {
        findings.extend(probe_findings(
            &loads,
            crate::probe::ProbeBudget::from_base(probe_timeout),
        ));
    }

    if ctx.json {
        let findings_json: Vec<Value> = findings
            .iter()
            .map(|f| {
                json!({
                    "severity": severity_label(f.severity),
                    "tool": f.tool.as_str(),
                    "server": f.server,
                    "message": f.message,
                    "fix": fix_hint(&f.message),
                })
            })
            .collect();
        let critical = findings
            .iter()
            .filter(|f| f.severity == Severity::Critical)
            .count();
        let warnings = findings
            .iter()
            .filter(|f| f.severity == Severity::Warning)
            .count();
        let info = findings
            .iter()
            .filter(|f| f.severity == Severity::Info)
            .count();
        print_json(&json!({
            "schema_version": 1,
            "findings": findings_json,
            "summary": {
                "critical": critical,
                "warning": warnings,
                "info": info,
            }
        }));
        return if critical > 0 || (strict && warnings > 0) {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        };
    }

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
    for spec in ctx.specs() {
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
    if explain && !findings.is_empty() {
        println!();
        println!("  How to fix:");
        let mut shown: Vec<&str> = Vec::new();
        for f in &findings {
            let Some(hint) = fix_hint(&f.message) else {
                continue;
            };
            if !shown.contains(&hint) {
                println!("    • {hint}");
                shown.push(hint);
            }
        }
    }
    if by_severity[0] > 0 || (strict && by_severity[1] > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

/// Apply safe repairs to every loaded (and editable) JSON config. Returns
/// whether anything was printed, or an error if persisting failed.
/// Codex TOML has no safe auto-repairs yet and is skipped.
fn apply_repairs(
    ctx: &Ctx,
    specs: &[&'static ToolSpec],
    loads: &mut [ToolLoad],
    dry_run: bool,
) -> Result<bool, String> {
    let mut printed = false;
    for (spec, load) in specs.iter().zip(loads) {
        let ConfigState::Loaded(cfg) = &mut load.state else {
            continue;
        };
        if matches!(cfg.raw, RawDoc::Toml(_)) {
            printed |= apply_toml_repairs(ctx, spec, cfg, dry_run)?;
            continue;
        }
        let RawDoc::Json(doc) = &mut cfg.raw else {
            continue;
        };
        if !cfg.editable {
            let mut preview = doc.clone();
            let possible = format_fix::fix_json_config(spec.id, spec.format, &mut preview);
            if !possible.is_empty() {
                println!(
                    "  {} {} config is read-only — {} repair(s) skipped",
                    report::glyph_warn(),
                    spec.display,
                    possible.len()
                );
                printed = true;
            }
            continue;
        }
        if dry_run {
            let mut preview = doc.clone();
            for repair in format_fix::fix_json_config(spec.id, spec.format, &mut preview) {
                println!("  ✚ would fix — {repair}");
                printed = true;
            }
            continue;
        }
        let fixes = format_fix::fix_json_config(spec.id, spec.format, doc);
        if fixes.is_empty() {
            continue;
        }
        for repair in &fixes {
            println!("  ✚ fixed — {repair}");
        }
        if let Some(backup) = commit(ctx, spec, &cfg.raw, false)? {
            println!("      backup: {}", backup.display());
        }
        printed = true;
    }
    Ok(printed)
}

/// Apply safe repairs to a loaded Codex TOML config. Returns whether
/// anything was printed, or an error if persisting failed.
fn apply_toml_repairs(
    ctx: &Ctx,
    spec: &ToolSpec,
    cfg: &mut crate::store::LoadedConfig,
    dry_run: bool,
) -> Result<bool, String> {
    let RawDoc::Toml(doc) = &mut cfg.raw else {
        return Ok(false);
    };
    if dry_run {
        let mut preview = doc.clone();
        let mut printed = false;
        for repair in format_toml::fix_toml_config(&mut preview) {
            println!("  ✚ would fix — {repair}");
            printed = true;
        }
        return Ok(printed);
    }
    let fixes = format_toml::fix_toml_config(doc);
    if fixes.is_empty() {
        return Ok(false);
    }
    for repair in &fixes {
        println!("  ✚ fixed — {repair}");
    }
    if let Some(backup) = commit(ctx, spec, &cfg.raw, false)? {
        println!("      backup: {}", backup.display());
    }
    Ok(true)
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Critical => "critical",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}
