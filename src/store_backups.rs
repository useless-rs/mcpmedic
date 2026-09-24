//! Automatic backups: directory layout, listing (newest first), and
//! the entry type restores select from.
//!
//! Split out of [`crate::store`].

use std::fs;
use std::path::{Path, PathBuf};

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
