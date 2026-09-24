//! Diff command: cross-tool drift reports with `--exit-code` CI gating.
//!
//! Split out of [`crate::commands_doctor`].

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::commands_ctx::{Ctx, display_path, fail, print_json, resolve};
use crate::diff;
use crate::model::Servers;
use crate::registry::ToolSpec;
use crate::report;
use crate::store::ConfigState;

pub(crate) fn cmd_diff(ctx: &Ctx, a: &str, b: &str, exit_code: bool) -> ExitCode {
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

    if ctx.json {
        let in_sync =
            result.only_a.is_empty() && result.only_b.is_empty() && result.different.is_empty();
        let different: Vec<Value> = result
            .different
            .iter()
            .map(|(name, reason)| json!({"name": name, "reason": reason}))
            .collect();
        print_json(&json!({
            "a": spec_a.id.as_str(),
            "b": spec_b.id.as_str(),
            "a_servers": servers_a.len(),
            "b_servers": servers_b.len(),
            "only_a": result.only_a,
            "only_b": result.only_b,
            "different": different,
            "identical": result.identical,
            "in_sync": in_sync,
        }));
        return drift_exit(in_sync, exit_code);
    }

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
    let in_sync =
        result.only_a.is_empty() && result.only_b.is_empty() && result.different.is_empty();
    if in_sync {
        println!("  ✓ no drift — both tools are in sync");
    }
    drift_exit(in_sync, exit_code)
}

/// Map drift state to an exit code: 0 normally, 1 when `--exit-code` is set
/// and the two tools disagree — so dotfile CI can gate on tool parity.
fn drift_exit(in_sync: bool, exit_code: bool) -> ExitCode {
    if exit_code && !in_sync {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
