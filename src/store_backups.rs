//! Automatic backups: directory layout, listing (newest first), and
//! the entry type restores select from.
//!
//! Split out of [`crate::store`].

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Directory where `mcpmedic` keeps automatic backups.
pub(crate) fn backups_dir(home: &Path) -> PathBuf {
    home.join(".mcpmedic").join("backups")
}

/// Build a backup path that can never collide: the pid and a process-wide
/// counter disambiguate two backups of one tool within the same millisecond.
pub(crate) fn backup_path(dir: &Path, stem: &str, ext: &str, millis: u128) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    dir.join(format!("{stem}-{millis}-{}-{n}.{ext}", std::process::id()))
}

/// Split a backup stem into its tool id and millis. Accepts the current
/// `{tool}-{millis}-{pid}-{counter}` shape and the legacy `{tool}-{millis}`
/// shape, so restores keep working across the upgrade.
fn parse_backup_stem(stem: &str) -> Option<(String, u128)> {
    let rev: Vec<&str> = stem.rsplit('-').collect();
    if rev.len() >= 4 {
        if let (Ok(_), Ok(_), Ok(millis)) = (
            rev[0].parse::<u64>(),
            rev[1].parse::<u32>(),
            rev[2].parse::<u128>(),
        ) {
            let tool: Vec<&str> = rev[3..].iter().rev().copied().collect();
            return Some((tool.join("-"), millis));
        }
    }
    if rev.len() >= 2 {
        if let Ok(millis) = rev[0].parse::<u128>() {
            let tool: Vec<&str> = rev[1..].iter().rev().copied().collect();
            return Some((tool.join("-"), millis));
        }
    }
    None
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
        let Some((tool_id, millis)) = parse_backup_stem(stem) else {
            continue;
        };
        entries.push(BackupEntry {
            tool_id,
            path: file.path(),
            millis,
        });
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.millis));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_paths_are_unique_and_parseable() {
        let dir = Path::new("/tmp");
        let a = backup_path(dir, "cursor", "json", 1000);
        let b = backup_path(dir, "cursor", "json", 1000);
        assert_ne!(a, b);
        for p in [a, b] {
            let stem = p.file_stem().unwrap().to_string_lossy().to_string();
            assert_eq!(parse_backup_stem(&stem), Some(("cursor".to_owned(), 1000)));
        }
    }

    #[test]
    fn legacy_stems_still_parse() {
        assert_eq!(
            parse_backup_stem("cursor-1000"),
            Some(("cursor".to_owned(), 1000))
        );
        assert_eq!(
            parse_backup_stem("claude-code-2000"),
            Some(("claude-code".to_owned(), 2000))
        );
        assert!(parse_backup_stem("garbage").is_none());
        assert!(parse_backup_stem("cursor-notanumber").is_none());
    }
}
