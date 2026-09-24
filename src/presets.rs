//! Curated MCP server bundles that work with zero configuration.

pub(crate) struct PresetServer {
    pub(crate) name: &'static str,
    pub(crate) command: &'static str,
    pub(crate) args: &'static [&'static str],
}

pub(crate) struct Preset {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) servers: &'static [PresetServer],
}

pub(crate) const PRESETS: &[Preset] = &[
    Preset {
        name: "minimal",
        description: "Zero-config starter set — knowledge-graph memory and structured step-by-step reasoning",
        servers: &[
            PresetServer {
                name: "memory",
                command: "npx",
                args: &["-y", "@modelcontextprotocol/server-memory"],
            },
            PresetServer {
                name: "sequential-thinking",
                command: "npx",
                args: &["-y", "@modelcontextprotocol/server-sequentialthinking"],
            },
        ],
    },
    Preset {
        name: "demo",
        description: "The official MCP reference test server — prompts, resources and many tools, perfect for exercising doctor --probe",
        servers: &[PresetServer {
            name: "everything",
            command: "npx",
            args: &["-y", "@modelcontextprotocol/server-everything"],
        }],
    },
];

pub(crate) fn find(name: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_unique_and_nonempty() {
        let mut names = Vec::new();
        for p in PRESETS {
            assert!(!p.name.is_empty(), "preset name must not be empty");
            assert!(!p.description.is_empty());
            assert!(!p.servers.is_empty(), "{} has no servers", p.name);
            assert!(!names.contains(&p.name), "duplicate preset {}", p.name);
            names.push(p.name);
            for s in p.servers {
                assert!(!s.name.is_empty(), "{} has empty server name", p.name);
                assert!(!s.command.is_empty(), "{} has empty command", s.name);
                assert!(!s.args.is_empty(), "{} has empty args", s.name);
            }
        }
    }

    #[test]
    fn finds_presets_by_name() {
        assert!(find("minimal").is_some());
        assert!(find("demo").is_some());
        assert!(find("nope").is_none());
    }
}
