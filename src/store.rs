//! Loading and persisting tool configs. Every mutation is preceded by an
//! automatic backup and applied with an atomic write (tmp file + rename) so a
//! crash can never leave a half-written config behind.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::format::{self, Format};
use crate::model::Servers;
use crate::registry::{EnvOverrides, ToolSpec};

/// A parsed config document, kept for surgical edits.
pub(crate) enum RawDoc {
    /// Any JSON dialect.
    Json(Value),
    /// Codex `config.toml`, with comments and formatting preserved.
    Toml(toml_edit::DocumentMut),
}

impl RawDoc {
    /// Render the document back to its file representation.
    pub(crate) fn serialize(&self) -> String {
        match self {
            Self::Json(v) => {
                let mut s = serde_json::to_string_pretty(v).unwrap_or_else(|_| "{}".into());
                s.push('\n');
                s
            }
            Self::Toml(doc) => doc.to_string(),
        }
    }
}

/// A successfully loaded config.
pub(crate) struct LoadedConfig {
    /// Parsed document for editing.
    pub raw: RawDoc,
    /// Normalized servers.
    pub servers: Servers,
    /// Names of servers parked by their tool's disable flag.
    pub disabled: BTreeSet<String>,
    /// Entry-level problems found while parsing (shown by `doctor`).
    pub problems: Vec<String>,
    /// Whether `mcpmedic` will agree to edit this file.
    pub editable: bool,
}

/// Everything that can happen to a tool's config on disk.
pub(crate) enum ConfigState {
    /// No config file exists.
    Missing,
    /// The file exists but cannot be parsed.
    ParseError(String),
    /// Parsed successfully.
    Loaded(Box<LoadedConfig>),
}

/// Load one tool's config relative to `home`.
pub(crate) fn load(spec: &ToolSpec, home: &Path, env: &EnvOverrides) -> ConfigState {
    let path = (spec.path)(home, env);
    if !path.exists() {
        return ConfigState::Missing;
    }
    let contents = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return ConfigState::ParseError(format!("cannot read file: {e}")),
    };
    if spec.format == Format::CodexToml {
        return match contents.parse::<toml_edit::DocumentMut>() {
            Ok(doc) => {
                let (servers, problems) = format::toml_servers(&doc);
                let disabled = spec
                    .disable
                    .as_ref()
                    .map_or_else(BTreeSet::new, |flag| format::toml_disabled(flag, &doc));
                ConfigState::Loaded(Box::new(LoadedConfig {
                    raw: RawDoc::Toml(doc),
                    servers,
                    disabled,
                    problems,
                    editable: true,
                }))
            }
            Err(e) => ConfigState::ParseError(e.to_string()),
        };
    }
    match serde_json::from_str::<Value>(&contents) {
        Ok(doc) => {
            let (servers, problems) = read_json_servers(spec.format, &doc);
            let disabled = spec.disable.as_ref().map_or_else(BTreeSet::new, |flag| {
                format::json_disabled(flag, spec.format, &doc)
            });
            ConfigState::Loaded(Box::new(LoadedConfig {
                raw: RawDoc::Json(doc),
                servers,
                disabled,
                problems,
                editable: spec.writable,
            }))
        }
        Err(strict_err) => {
            // JSONC tolerance (Zed settings.json, opencode): parse a stripped
            // copy for reading, but never rewrite the file — that would
            // destroy the user's comments.
            let stripped = format::jsonc_strip(&contents);
            match serde_json::from_str::<Value>(&stripped) {
                Ok(doc) => {
                    let (servers, problems) = read_json_servers(spec.format, &doc);
                    let disabled = spec.disable.as_ref().map_or_else(BTreeSet::new, |flag| {
                        format::json_disabled(flag, spec.format, &doc)
                    });
                    ConfigState::Loaded(Box::new(LoadedConfig {
                        raw: RawDoc::Json(doc),
                        servers,
                        disabled,
                        problems,
                        editable: false,
                    }))
                }
                Err(_) => ConfigState::ParseError(strict_err.to_string()),
            }
        }
    }
}

fn read_json_servers(format: Format, doc: &Value) -> (Servers, Vec<String>) {
    match format {
        Format::Opencode => format::opencode_servers(doc),
        _ => format::json_servers(format, doc),
    }
}

/// Directory where `mcpmedic` keeps automatic backups.
pub(crate) fn backups_dir(home: &Path) -> PathBuf {
    home.join(".mcpmedic").join("backups")
}

/// Atomically persist `contents` to `path`, backing up the previous version
/// first. Returns the backup path when one was created.
pub(crate) fn persist(
    path: &Path,
    contents: &str,
    backup_stem: &str,
    home: &Path,
) -> std::io::Result<Option<PathBuf>> {
    let backup = if path.exists() {
        let dir = backups_dir(home);
        fs::create_dir_all(&dir)?;
        let millis = epoch_millis();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("json");
        let target = dir.join(format!("{backup_stem}-{millis}.{ext}"));
        fs::copy(path, &target)?;
        Some(target)
    } else {
        None
    };

    let tmp = path.with_extension(format!("mcpmedic-{}.tmp", epoch_millis()));
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(backup)
}

fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_config_is_reported() {
        let env = EnvOverrides::default();
        let spec = crate::registry::registry()
            .iter()
            .find(|s| s.id == crate::registry::ToolId::Cursor)
            .unwrap();
        let home = std::env::temp_dir().join(format!("mcpmedic-unit-{}", std::process::id()));
        assert!(matches!(load(spec, &home, &env), ConfigState::Missing));
    }

    #[test]
    fn jsonc_config_is_loaded_but_not_editable() {
        let home = std::env::temp_dir().join(format!("mcpmedic-jsonc-{}", std::process::id()));
        let cursor_dir = home.join(".cursor");
        fs::create_dir_all(&cursor_dir).unwrap();
        fs::write(
            cursor_dir.join("mcp.json"),
            "{\n  // favorites\n  \"mcpServers\": {\"c7\": {\"command\": \"npx\"}}\n}",
        )
        .unwrap();

        let env = EnvOverrides::default();
        let spec = crate::registry::registry()
            .iter()
            .find(|s| s.id == crate::registry::ToolId::Cursor)
            .unwrap();
        let ConfigState::Loaded(cfg) = load(spec, &home, &env) else {
            panic!("expected loaded config");
        };
        assert!(!cfg.editable, "commented configs must not be editable");
        assert_eq!(cfg.servers.len(), 1);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn persist_creates_backup_and_replaces_content() {
        let home = std::env::temp_dir().join(format!("mcpmedic-persist-{}", std::process::id()));
        let dir = home.join(".cursor");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mcp.json");
        fs::write(&path, "{\"mcpServers\":{}}").unwrap();

        let backup = persist(&path, "{\"mcpServers\":{\"a\":{}}}", "cursor", &home)
            .unwrap()
            .expect("backup expected");
        assert!(backup.exists());
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"a\""));
        let _ = fs::remove_dir_all(&home);
    }
}
