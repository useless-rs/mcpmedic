//! Audit command: hardcoded-secret scan + config permission warnings.
//!
//! Split out of [`crate::commands`].

use std::process::ExitCode;

use serde_json::json;

use crate::audit;
use crate::commands_ctx::{Ctx, fail, print_json, resolve};
use crate::registry::ToolSpec;
use crate::report;
use crate::store::ConfigState;

pub(crate) fn cmd_audit(ctx: &Ctx, tool: Option<&str>, strict: bool) -> ExitCode {
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
        return if secrets > 0 || (strict && perms_warnings > 0) {
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
    } else if strict && perms_warnings > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
