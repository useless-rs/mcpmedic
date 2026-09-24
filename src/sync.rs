//! Sync planning: additive merges between tools. `mcpmedic` never deletes
//! servers in the target — it only adds missing ones, and overwrites drifted
//! ones when `--force` is passed.

use crate::model::{Servers, Transport};

/// What a sync would do.
pub(crate) struct SyncPlan {
    /// Servers missing in the target that will be copied.
    pub to_add: Vec<(String, Transport)>,
    /// Servers that exist in both with different configs (skipped unless `--force`).
    pub drifted: Vec<String>,
    /// Servers already identical in both (always skipped).
    pub identical: Vec<String>,
    /// Names filtered out because they do not exist in the source.
    pub unknown: Vec<String>,
}

/// Plan a sync from `from` into `to`.
///
/// * `names` — when non-empty, only these servers are considered.
/// * `force` — when true, drifted servers are moved into `to_add`.
pub(crate) fn plan(from: &Servers, to: &Servers, names: &[String], force: bool) -> SyncPlan {
    let filter = |name: &str| names.is_empty() || names.iter().any(|n| n == name);

    let mut to_add = Vec::new();
    let mut drifted = Vec::new();
    let mut identical = Vec::new();
    let mut unknown = Vec::new();

    for requested in names {
        if !from.contains_key(requested) && !unknown.contains(requested) {
            unknown.push(requested.clone());
        }
    }

    for (name, transport) in from {
        if !filter(name) {
            continue;
        }
        match to.get(name) {
            None => to_add.push((name.clone(), transport.clone())),
            Some(existing) => {
                if existing == transport {
                    identical.push(name.clone());
                } else {
                    if force {
                        to_add.push((name.clone(), transport.clone()));
                    }
                    drifted.push(name.clone());
                }
            }
        }
    }

    SyncPlan {
        to_add,
        drifted,
        identical,
        unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn stdio(command: &str) -> Transport {
        Transport::Stdio {
            command: command.into(),
            args: vec![],
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn plan_adds_missing_and_reports_drift() {
        let mut from = Servers::new();
        from.insert("new".into(), stdio("npx"));
        from.insert("same".into(), stdio("echo"));
        from.insert("drift".into(), stdio("npx"));
        let mut to = Servers::new();
        to.insert("same".into(), stdio("echo"));
        to.insert("drift".into(), stdio("bun"));
        to.insert("extra-kept".into(), stdio("keep"));

        let p = plan(&from, &to, &[], false);
        assert_eq!(p.to_add.len(), 1);
        assert_eq!(p.to_add[0].0, "new");
        assert_eq!(p.drifted, vec!["drift".to_owned()]);
        assert_eq!(p.identical, vec!["same".to_owned()]);
        assert!(p.unknown.is_empty());

        // force moves drifted into to_add
        let p = plan(&from, &to, &[], true);
        assert_eq!(p.to_add.len(), 2);
        assert!(p.drifted.contains(&"drift".to_owned()));
    }

    #[test]
    fn name_filter_and_unknown_names() {
        let mut from = Servers::new();
        from.insert("a".into(), stdio("x"));
        from.insert("b".into(), stdio("y"));
        let to = Servers::new();

        let p = plan(&from, &to, &["a".to_owned(), "zzz".to_owned()], false);
        assert_eq!(p.to_add.len(), 1);
        assert_eq!(p.to_add[0].0, "a");
        assert_eq!(p.unknown, vec!["zzz".to_owned()]);
    }

    #[test]
    fn unknown_names_are_deduped() {
        let from = Servers::new();
        let to = Servers::new();
        let p = plan(&from, &to, &["zzz".to_owned(), "zzz".to_owned()], false);
        assert_eq!(p.unknown, vec!["zzz".to_owned()]);
    }

    #[test]
    fn target_servers_are_never_deleted() {
        let from = Servers::new();
        let mut to = Servers::new();
        to.insert("only-in-target".into(), stdio("keep"));
        let p = plan(&from, &to, &[], false);
        assert!(p.to_add.is_empty());
        assert!(to.contains_key("only-in-target"));
    }
}
