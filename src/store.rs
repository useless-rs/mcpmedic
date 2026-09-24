//! Loading and persisting tool configs. Every mutation is preceded by an
//! automatic backup and applied with an atomic write (tmp file + rename) so a
//! crash can never leave a half-written config behind.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::format::{self, Format};
use crate::format_toml;
use crate::format_yaml;
use crate::model::Servers;
use crate::registry::ToolSpec;

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

/// Load one tool's config from an explicit path.
pub(crate) fn load(spec: &ToolSpec, path: &Path) -> ConfigState {
    if !path.exists() {
        return ConfigState::Missing;
    }
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return ConfigState::ParseError(format!("cannot read file: {e}")),
    };
    if spec.format == Format::Yaml {
        return match crate::yaml::parse_yaml(&contents) {
            Ok(doc) => {
                let (servers, problems) = format_yaml::yaml_servers(spec.id, &doc);
                let disabled = spec.disable.as_ref().map_or_else(BTreeSet::new, |flag| {
                    format_yaml::yaml_disabled(spec.id, flag, &doc)
                });
                ConfigState::Loaded(Box::new(LoadedConfig {
                    raw: RawDoc::Json(doc),
                    servers,
                    disabled,
                    problems,
                    editable: false,
                }))
            }
            Err(e) => ConfigState::ParseError(e),
        };
    }
    if spec.format == Format::CodexToml {
        return match contents.parse::<toml_edit::DocumentMut>() {
            Ok(doc) => {
                let (servers, problems) = format_toml::toml_servers(&doc);
                let disabled = spec
                    .disable
                    .as_ref()
                    .map_or_else(BTreeSet::new, |flag| format_toml::toml_disabled(flag, &doc));
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
            let disabled = disabled_set(spec, &doc);
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
                    let disabled = disabled_set(spec, &doc);
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
        Format::OpenClaw => format::openclaw_servers(doc),
        _ => format::json_servers(format, doc),
    }
}

fn disabled_set(spec: &ToolSpec, doc: &Value) -> BTreeSet<String> {
    match (spec.disable.as_ref(), spec.format) {
        (Some(flag), Format::OpenClaw) => format::openclaw_disabled(flag, doc),
        (Some(flag), _) => format::json_disabled(flag, spec.format, doc),
        (None, _) => BTreeSet::new(),
    }
}

/// Directory where `mcpmedic` keeps automatic backups.
pub(crate) fn backups_dir(home: &Path) -> PathBuf {
    home.join(".mcpmedic").join("backups")
}

/// One backup file on disk.
pub(crate) struct BackupEntry {
    /// Tool the backup belongs to.
    pub tool_id: String,
    /// Path to the backup file.
    pub path: PathBuf,
    /// Creation time in epoch milliseconds.
    pub millis: u128,
}

/// List all backups in `~/.mcpmedic/backups/`, newest first.
pub(crate) fn list_backups(home: &Path) -> Vec<BackupEntry> {
    let mut entries = Vec::new();
    let Ok(files) = fs::read_dir(backups_dir(home)) else {
        return entries;
    };
    for file in files.flatten() {
        let name = file.file_name().to_string_lossy().to_string();
        let Some((stem, _)) = name.rsplit_once('.') else {
            continue;
        };
        let Some((tool_id, millis_str)) = stem.rsplit_once('-') else {
            continue;
        };
        let Ok(millis) = millis_str.parse::<u128>() else {
            continue;
        };
        entries.push(BackupEntry {
            tool_id: tool_id.to_owned(),
            path: file.path(),
            millis,
        });
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.millis));
    entries
}

/// Atomically persist `contents` to `path`, backing up the previous version
/// first. Returns the backup path when one was created.
pub(crate) fn persist(
    path: &Path,
    contents: &str,
    backup_stem: &str,
    home: &Path,
) -> std::io::Result<Option<PathBuf>> {
    let original_perms = if path.exists() {
        fs::metadata(path).ok().map(|m| m.permissions())
    } else {
        None
    };
    let backup = if path.exists() {
        let dir = backups_dir(home);
        fs::create_dir_all(&dir)?;
        restrict_dir_unix(&dir);
        let millis = epoch_millis();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("json");
        let target = dir.join(format!("{backup_stem}-{millis}.{ext}"));
        fs::copy(path, &target)?;
        restrict_file_unix(&target);
        Some(target)
    } else {
        None
    };

    let tmp = tmp_path(path);
    if let Err(e) = fs::write(&tmp, contents) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    apply_config_perms_unix(&tmp, original_perms);
    if let Err(e) = sync_tmp(&tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(backup)
}

/// Atomically write `contents` to an arbitrary path (no backup): tmp
/// sibling + fsync + rename, with tmp cleanup on failure. For export
/// targets and other non-config files that must never be half-written.
pub(crate) fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    let tmp = tmp_path(path);
    if let Err(e) = fs::write(&tmp, contents) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = sync_tmp(&tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Collision-proof sibling tmp path: same directory (so rename stays atomic),
/// unique per process + millisecond + counter so two persists in the same
/// millisecond — or a stale tmp from a crashed run — can never clobber.
fn tmp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!(
        "mcpmedic-{}-{}-{n}.tmp",
        std::process::id(),
        epoch_millis()
    ))
}

/// Flush tmp file contents to the OS before rename so a crash can never leave
/// a truncated config behind.
fn sync_tmp(tmp: &Path) -> std::io::Result<()> {
    let f = fs::File::open(tmp)?;
    f.sync_all()
}

#[cfg(unix)]
fn restrict_dir_unix(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_dir_unix(_dir: &Path) {}

#[cfg(unix)]
fn restrict_file_unix(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_file_unix(_path: &Path) {}

#[cfg(unix)]
fn apply_config_perms_unix(path: &Path, original: Option<fs::Permissions>) {
    use std::os::unix::fs::PermissionsExt;
    let mode = original.map_or(0o600, |p| p.mode());
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn apply_config_perms_unix(_path: &Path, _original: Option<fs::Permissions>) {}

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
        let spec = crate::registry::registry()
            .iter()
            .find(|s| s.id == crate::registry::ToolId::Cursor)
            .unwrap();
        let home = std::env::temp_dir().join(format!("mcpmedic-unit-{}", std::process::id()));
        assert!(matches!(
            load(spec, &home.join(".cursor/mcp.json")),
            ConfigState::Missing
        ));
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

        let spec = crate::registry::registry()
            .iter()
            .find(|s| s.id == crate::registry::ToolId::Cursor)
            .unwrap();
        let ConfigState::Loaded(cfg) = load(spec, &cursor_dir.join("mcp.json")) else {
            panic!("expected loaded config");
        };
        assert!(!cfg.editable, "commented configs must not be editable");
        assert_eq!(cfg.servers.len(), 1);
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    fn tmp_paths_are_unique_within_a_process() {
        let base = Path::new("/tmp/mcp.json");
        let a = tmp_path(base);
        let b = tmp_path(base);
        assert_ne!(a, b);
        assert_eq!(a.parent(), b.parent());
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
