//! Security audit: hardcoded secrets in MCP configs and config file
//! permissions. Detection is from scratch — prefix signatures borrowed from
//! the gitleaks/trufflehog rule sets plus a Shannon-entropy heuristic — so
//! the binary keeps zero new dependencies.

use std::collections::BTreeMap;
use std::path::Path;

use crate::model::Transport;

/// One hardcoded-secret finding.
#[derive(Debug)]
pub(crate) struct SecretFinding {
    /// Server the secret lives in.
    pub server: String,
    /// Where inside the entry, e.g. "env `NAME`" or "header `Name`".
    pub location: String,
    /// Human label, e.g. "GitHub token".
    pub label: &'static str,
}

/// Prefix signatures ordered longest-first so `sk_live_` wins over `sk-`.
/// (prefix, label, minimum total length)
const PREFIX_SIGNATURES: &[(&str, &str, usize)] = &[
    ("github_pat_", "GitHub fine-grained token", 30),
    ("sk_live_", "Stripe secret key", 24),
    ("sk_test_", "Stripe secret key", 24),
    ("rk_live_", "Stripe restricted key", 24),
    ("rk_test_", "Stripe restricted key", 24),
    ("sk-ant-", "Anthropic API key", 30),
    ("sk-", "OpenAI-style API key", 24),
    ("ghp_", "GitHub token", 40),
    ("gho_", "GitHub OAuth token", 40),
    ("ghu_", "GitHub app token", 40),
    ("ghs_", "GitHub server token", 40),
    ("ghr_", "GitHub refresh token", 40),
    ("xoxb-", "Slack bot token", 20),
    ("xoxp-", "Slack user token", 20),
    ("xoxa-", "Slack app token", 20),
    ("xoxr-", "Slack refresh token", 20),
    ("xapp-", "Slack app token", 20),
    ("AKIA", "AWS access key", 20),
    ("ASIA", "AWS temporary key", 20),
    ("ABIA", "AWS service key", 20),
    ("ACCA", "AWS cross-account key", 20),
    ("AIza", "Google API key", 39),
];

/// Field-name fragments that mark a value as credential-shaped.
const NAME_HINTS: &[&str] = &[
    "key",
    "token",
    "secret",
    "password",
    "passwd",
    "pwd",
    "credential",
    "auth",
];

/// Shannon entropy (bits/char) of a value — real tokens sit high, words low.
fn shannon_entropy(value: &str) -> f64 {
    let mut counts: BTreeMap<char, usize> = BTreeMap::new();
    for c in value.chars() {
        *counts.entry(c).or_default() += 1;
    }
    let total = f64::from(u32::try_from(value.chars().count()).unwrap_or(u32::MAX));
    counts
        .values()
        .map(|&n| {
            let p = f64::from(u32::try_from(n).unwrap_or(u32::MAX)) / total;
            -p * p.log2()
        })
        .sum()
}

/// Placeholder values that must never be reported: templated variables,
/// documentation stubs and obvious dummies.
fn is_placeholder(value: &str) -> bool {
    let v = value.trim();
    if v.contains('$') || v.contains('%') || v.contains('<') || v.contains('{') {
        return true;
    }
    let lower = v.to_ascii_lowercase();
    [
        "your",
        "example",
        "placeholder",
        "changeme",
        "sample",
        "dummy",
        "xxxx",
    ]
    .iter()
    .any(|stub| lower.starts_with(stub))
}

fn classify(value: &str) -> Option<&'static str> {
    for (prefix, label, min_len) in PREFIX_SIGNATURES {
        if value.starts_with(prefix) && value.chars().count() >= *min_len {
            return Some(label);
        }
    }
    if value.starts_with("eyJ") && value.matches('.').count() >= 2 {
        return Some("JWT literal");
    }
    if value.contains("-----BEGIN") {
        return Some("private key");
    }
    None
}

fn scan_field(
    server: &str,
    kind: &str,
    name: &str,
    value: &str,
    findings: &mut Vec<SecretFinding>,
) {
    if is_placeholder(value) {
        return;
    }
    if let Some(label) = classify(value) {
        findings.push(SecretFinding {
            server: server.to_owned(),
            location: format!("{kind} `{name}`"),
            label,
        });
        return;
    }
    let lower_name = name.to_ascii_lowercase();
    let credential_shaped = NAME_HINTS.iter().any(|hint| lower_name.contains(hint));
    if credential_shaped && value.chars().count() >= 16 && shannon_entropy(value) >= 3.5 {
        findings.push(SecretFinding {
            server: server.to_owned(),
            location: format!("{kind} `{name}`"),
            label: "high-entropy credential",
        });
    }
}

/// Scan one normalized server entry for hardcoded secrets in its env or
/// header values.
pub(crate) fn scan_transport(server: &str, transport: &Transport) -> Vec<SecretFinding> {
    let mut findings = Vec::new();
    match transport {
        Transport::Stdio { env, .. } => {
            scan_map(server, "env", env, &mut findings);
        }
        Transport::Remote { headers, .. } => {
            scan_map(server, "header", headers, &mut findings);
            let literal_auth = headers
                .iter()
                .any(|(k, v)| k.eq_ignore_ascii_case("authorization") && !is_placeholder(v));
            if literal_auth {
                findings.push(SecretFinding {
                    server: server.to_owned(),
                    location: "header `Authorization`".to_owned(),
                    label: "literal credential in Authorization header",
                });
            }
        }
    }
    findings
}

fn scan_map(
    server: &str,
    kind: &str,
    map: &BTreeMap<String, String>,
    findings: &mut Vec<SecretFinding>,
) {
    for (name, value) in map {
        scan_field(server, kind, name, value, findings);
    }
}

/// Warn when a config file full of credentials is readable by group/others
/// (Unix). `None` on non-Unix platforms or when the stat fails.
pub(crate) fn permissions_warning(path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        let mode = fs_metadata_mode(path)?;
        if mode & 0o077 != 0 {
            return Some(format!(
                "readable by others (mode {:o}) — chmod 600 recommended",
                mode & 0o777
            ));
        }
        None
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(unix)]
fn fs_metadata_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).ok().map(|m| m.permissions().mode())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn flags_known_token_prefixes() {
        let transport = Transport::Stdio {
            command: "npx".into(),
            args: vec![],
            env: env_map(&[("GITHUB_PAT", "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB")]),
        };
        let findings = scan_transport("srv", &transport);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].label, "GitHub token");
    }

    #[test]
    fn flags_aws_openai_and_jwt() {
        let transport = Transport::Stdio {
            command: "npx".into(),
            args: vec![],
            env: env_map(&[
                ("AWS", "AKIAIOSFODNN7EXAMPLE2X"),
                ("OPENAI", "sk-proj-0123456789abcdefghijklmnopqrstuvwxyz"),
                (
                    "JWT",
                    "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV",
                ),
            ]),
        };
        let findings = scan_transport("srv", &transport);
        assert_eq!(findings.len(), 3, "{findings:?}");
    }

    #[test]
    fn skips_placeholders_and_templated_values() {
        let transport = Transport::Stdio {
            command: "npx".into(),
            args: vec![],
            env: env_map(&[
                ("API_KEY", "${MY_API_KEY}"),
                ("TOKEN", "your-token-here"),
                ("SECRET", "xxxxxxxxxxxxxxxx"),
            ]),
        };
        assert!(scan_transport("srv", &transport).is_empty());
    }

    #[test]
    fn generic_entropy_catches_unprefixed_credential() {
        let transport = Transport::Stdio {
            command: "npx".into(),
            args: vec![],
            env: env_map(&[("MY_SERVICE_API_KEY", "fJ29xKq8pLw3ZvQr7Tm4Nb6")]),
        };
        let findings = scan_transport("srv", &transport);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].label, "high-entropy credential");
    }

    #[test]
    fn entropy_separates_tokens_from_words() {
        assert!(shannon_entropy("correct horse battery staple") < 3.5);
        assert!(shannon_entropy("fJ29xKq8pLw3ZvQr7Tm4Nb6Vd") > 3.5);
    }

    #[test]
    fn authorization_header_literal_is_flagged() {
        let transport = Transport::Remote {
            url: "https://example.com/mcp".into(),
            headers: env_map(&[("Authorization", "Bearer abc123def456ghi789")]),
        };
        let findings = scan_transport("srv", &transport);
        assert!(
            findings.iter().any(|f| f.label.contains("Authorization")),
            "{findings:?}"
        );
    }
}
