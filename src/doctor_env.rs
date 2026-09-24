//! Environment helpers for health checks: command lookup on `PATH`
//! (with `~`, absolute/relative, and Windows handling) and `${VAR}` /
//! `${env:VAR}` reference extraction.
//!
//! Split out of [`crate::doctor`].

use std::path::Path;

/// Check whether a command exists: absolute/relative paths are checked on
/// disk, bare names are looked up in `PATH`. `~` is expanded with `home`.
pub(crate) fn command_exists(command: &str, path_env: &str, home: &Path) -> bool {
    if command.contains("${") || command.contains('%') {
        return true; // templated values are resolved by the tool itself
    }
    let expanded = if let Some(rest) = command.strip_prefix("~/") {
        home.join(rest)
    } else {
        Path::new(command).to_path_buf()
    };
    if command.contains('/') || command.contains('\\') {
        return expanded.exists();
    }
    let separator = if cfg!(windows) { ';' } else { ':' };
    path_env.split(separator).any(|dir| {
        let direct = Path::new(dir).join(command);
        if direct.exists() {
            return true;
        }
        if cfg!(windows) {
            let exe = Path::new(dir).join(format!("{command}.exe"));
            if exe.exists() {
                return true;
            }
            let cmd = Path::new(dir).join(format!("{command}.cmd"));
            return cmd.exists();
        }
        false
    })
}

/// Extract variable names from `${VAR}` or `${env:VAR}` template patterns
/// in a config value.
pub(crate) fn extract_env_var_refs(value: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '{' {
            let mut j = i + 2;
            let mut var = String::new();
            while j < chars.len() && chars[j] != '}' {
                var.push(chars[j]);
                j += 1;
            }
            if j < chars.len() {
                let name = var.strip_prefix("env:").unwrap_or(&var).to_string();
                let first_ok = name
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_');
                if first_ok
                    && name
                        .chars()
                        .skip(1)
                        .all(|c| c.is_alphanumeric() || c == '_')
                {
                    vars.push(name);
                }
                i = j + 1;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    vars
}
