//! Shared command plumbing: resolved context, JSON printing, errors.
//!
//! Split out of [`crate::commands`]. Every subcommand builds on this.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::Value;

use crate::registry::{EnvOverrides, ToolSpec};
use crate::report;
use crate::store::{self, ConfigState};

/// Everything commands need to operate, resolved once.
pub(crate) struct Ctx {
    pub home: PathBuf,
    pub env: EnvOverrides,
    pub project: Option<PathBuf>,
    pub json: bool,
    pub quiet: bool,
}

impl Ctx {
    pub(crate) fn new(project: Option<PathBuf>, json: bool, quiet: bool) -> Self {
        Self {
            home: home_dir(),
            env: EnvOverrides::from_env(),
            project: project.map(|dir| {
                if dir.is_absolute() {
                    dir
                } else {
                    std::env::current_dir().unwrap_or_default().join(dir)
                }
            }),
            json,
            quiet,
        }
    }

    pub(crate) fn path(&self, spec: &ToolSpec) -> PathBuf {
        if let (Some(project), Some(project_path)) = (&self.project, spec.project_path) {
            project_path(project)
        } else {
            (spec.path)(&self.home, &self.env)
        }
    }

    pub(crate) fn load(&self, spec: &'static ToolSpec) -> ConfigState {
        if self.project.is_some() && spec.project_path.is_none() {
            return ConfigState::Missing;
        }
        store::load(spec, &self.path(spec))
    }

    /// The specs participating in this mode: with `--project`, only tools
    /// that document a project-scoped config.
    pub(crate) fn specs(&self) -> Vec<&'static ToolSpec> {
        if self.project.is_some() {
            crate::registry::registry()
                .iter()
                .filter(|spec| spec.project_path.is_some())
                .collect()
        } else {
            crate::registry::registry().iter().collect()
        }
    }
}

/// Emit `value` as the single JSON document for this invocation.
pub(crate) fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
    );
}

pub(crate) fn home_dir() -> PathBuf {
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

pub(crate) fn fail(message: &str) -> ExitCode {
    eprintln!("{}", report::error_line(message));
    ExitCode::from(2)
}

pub(crate) fn resolve(name: &str) -> Result<&'static ToolSpec, String> {
    crate::registry::resolve_tool(name)
        .ok_or_else(|| format!("unknown tool `{name}` — run `mcpmedic scan` to list tools"))
}

/// Render a path relative to the home directory as `~/...` when possible.
pub(crate) fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => path.display().to_string(),
    }
}
