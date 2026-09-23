//! Diffing the server sets of two tools.

use crate::model::{Servers, Transport};

/// Result of comparing two tools' servers.
pub(crate) struct ToolDiff {
    /// Server names only present in the first tool.
    pub only_a: Vec<String>,
    /// Server names only present in the second tool.
    pub only_b: Vec<String>,
    /// Names present in both but with different configs, with the first
    /// difference found.
    pub different: Vec<(String, String)>,
    /// How many names are present and identical in both.
    pub identical: usize,
}

/// Compare two server maps.
pub(crate) fn diff(a: &Servers, b: &Servers) -> ToolDiff {
    let mut only_a = Vec::new();
    let mut only_b = Vec::new();
    let mut different = Vec::new();
    let mut identical = 0;

    for (name, ta) in a {
        match b.get(name) {
            None => only_a.push(name.clone()),
            Some(tb) => {
                if ta == tb {
                    identical += 1;
                } else {
                    different.push((name.clone(), first_difference(ta, tb)));
                }
            }
        }
    }
    for name in b.keys() {
        if !a.contains_key(name) {
            only_b.push(name.clone());
        }
    }

    ToolDiff {
        only_a,
        only_b,
        different,
        identical,
    }
}

/// Describe the first meaningful difference between two transports.
fn first_difference(a: &Transport, b: &Transport) -> String {
    match (a, b) {
        (
            Transport::Stdio { command, args, env },
            Transport::Stdio {
                command: command2,
                args: args2,
                env: env2,
            },
        ) => {
            if command != command2 {
                format!("`{command}` vs `{command2}`")
            } else if args != args2 {
                format!(
                    "args differ ({:?} vs {:?})",
                    args.join(" "),
                    args2.join(" ")
                )
            } else {
                format!("env differs ({} vs {} variables)", env.len(), env2.len())
            }
        }
        (
            Transport::Remote { url, headers },
            Transport::Remote {
                url: url2,
                headers: headers2,
            },
        ) => {
            if url == url2 {
                let mut changed: Vec<&str> = headers
                    .keys()
                    .chain(headers2.keys())
                    .filter(|k| headers.get(*k) != headers2.get(*k))
                    .map(String::as_str)
                    .collect();
                changed.sort_unstable();
                changed.dedup();
                if changed.is_empty() {
                    "headers differ".to_string()
                } else {
                    format!("headers differ ({})", changed.join(", "))
                }
            } else {
                format!("`{url}` vs `{url2}`")
            }
        }
        _ => format!("transport kind ({} vs {})", a.kind(), b.kind()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn stdio(command: &str, args: &[&str]) -> Transport {
        Transport::Stdio {
            command: command.into(),
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            env: BTreeMap::new(),
        }
    }

    #[test]
    fn diff_classifies_all_buckets() {
        let mut a = Servers::new();
        a.insert("shared".into(), stdio("npx", &[]));
        a.insert("drifted".into(), stdio("npx", &["-y", "one"]));
        a.insert("only-a".into(), stdio("echo", &[]));
        let mut b = Servers::new();
        b.insert("shared".into(), stdio("npx", &[]));
        b.insert("drifted".into(), stdio("npx", &["-y", "two"]));
        b.insert("only-b".into(), stdio("echo", &[]));

        let d = diff(&a, &b);
        assert_eq!(d.only_a, vec!["only-a".to_owned()]);
        assert_eq!(d.only_b, vec!["only-b".to_owned()]);
        assert_eq!(d.identical, 1);
        assert_eq!(d.different.len(), 1);
        assert_eq!(d.different[0].0, "drifted");
        assert!(d.different[0].1.contains("args differ"));
    }

    #[test]
    fn diff_names_differing_headers() {
        let mut a = Servers::new();
        a.insert(
            "docs".into(),
            Transport::Remote {
                url: "https://x/mcp".into(),
                headers: [("Authorization".to_owned(), "Bearer t".to_owned())].into(),
            },
        );
        let mut b = Servers::new();
        b.insert(
            "docs".into(),
            Transport::Remote {
                url: "https://x/mcp".into(),
                headers: BTreeMap::new(),
            },
        );

        let d = diff(&a, &b);
        assert_eq!(d.different.len(), 1);
        assert!(
            d.different[0].1.contains("headers differ"),
            "got: {}",
            d.different[0].1
        );
        assert!(
            d.different[0].1.contains("Authorization"),
            "got: {}",
            d.different[0].1
        );
    }
}
