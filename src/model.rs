//! Normalized, tool-agnostic representation of an MCP server.

use std::collections::BTreeMap;

/// Map of server name to its normalized [`Transport`].
pub(crate) type Servers = BTreeMap<String, Transport>;

/// How a server connects: a local stdio process or a remote HTTP endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Transport {
    /// A local process started over the stdio transport.
    Stdio {
        /// Executable to run.
        command: String,
        /// Arguments passed to the executable.
        args: Vec<String>,
        /// Environment variables for the process.
        env: BTreeMap<String, String>,
    },
    /// A remote streamable-HTTP endpoint.
    Remote {
        /// Endpoint URL.
        url: String,
        /// HTTP headers sent with every request.
        headers: BTreeMap<String, String>,
    },
}

impl Transport {
    /// Short human-readable rendering, e.g. `npx -y pkg` or a URL.
    pub(crate) fn summary(&self) -> String {
        match self {
            Self::Stdio { command, args, .. } => {
                if args.is_empty() {
                    command.clone()
                } else {
                    format!("{command} {}", args.join(" "))
                }
            }
            Self::Remote { url, .. } => url.clone(),
        }
    }

    /// Transport kind label used in tables: `stdio` or `remote`.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::Remote { .. } => "remote",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stdio_summary_joins_args() {
        let t = Transport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "pkg".into()],
            env: BTreeMap::new(),
        };
        assert_eq!(t.summary(), "npx -y pkg");
        assert_eq!(t.kind(), "stdio");
    }

    #[test]
    fn remote_summary_is_url() {
        let t = Transport::Remote {
            url: "https://example.com/mcp".into(),
            headers: BTreeMap::new(),
        };
        assert_eq!(t.summary(), "https://example.com/mcp");
        assert_eq!(t.kind(), "remote");
    }
}
