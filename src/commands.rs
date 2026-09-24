//! Command implementations: the glue between the CLI and the engine.

use std::process::ExitCode;

use clap::CommandFactory;
use serde_json::json;

use crate::cli::Cmd;
use crate::commands_audit::cmd_audit;
use crate::commands_ctx::{Ctx, display_path, fail, print_json, resolve};
use crate::commands_diff::cmd_diff;
use crate::commands_doctor::cmd_doctor;
use crate::commands_mutate::{cmd_add, cmd_preset_add, cmd_preset_list, cmd_rm, cmd_set_enabled};
use crate::commands_portable::{cmd_backup, cmd_export, cmd_import, cmd_restore};
use crate::commands_read::{cmd_list, cmd_scan, cmd_show};
use crate::commands_status::{cmd_init, cmd_summary};
use crate::commands_sync::{cmd_sync, cmd_sync_all};

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
