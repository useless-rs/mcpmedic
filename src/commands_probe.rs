//! Probe fan-out: run reachability probes for every enabled server in
//! parallel threads and convert outcomes into doctor findings.
//!
//! Split out of [`crate::commands_doctor`].

use crate::doctor::{self, Severity, ToolLoad};
use crate::store::ConfigState;

pub(crate) fn probe_findings(
    loads: &[ToolLoad],
    budget: crate::probe::ProbeBudget,
) -> Vec<doctor::Finding> {
    let mut jobs = Vec::new();
    for load in loads {
        let ConfigState::Loaded(cfg) = &load.state else {
            continue;
        };
        for (name, transport) in &cfg.servers {
            if cfg.disabled.contains(name) {
                continue;
            }
            jobs.push((load.id, name.clone(), transport));
        }
    }
    std::thread::scope(|s| {
        let handles: Vec<_> = jobs
            .iter()
            .map(|(tool, name, transport)| {
                (*tool, name.clone(), s.spawn(move || {
                    let (severity, message) = match crate::probe::probe_transport(transport, budget) {
                        crate::probe::Probe::McpOk {
                            server,
                            protocol,
                            tools,
                        } => {
                            let tools_note = tools.map_or(String::new(), |n| {
                                format!(", exposes {n} tool(s)")
                            });
                            (
                                Severity::Info,
                                format!(
                                    "probe: MCP handshake ok — `{server}` speaks protocol {protocol}{tools_note}"
                                ),
                            )
                        }
                        crate::probe::Probe::ModernOk {
                            server,
                            versions,
                            tools,
                        } => {
                            let versions_list = versions.join(", ");
                            let tools_note = tools.map_or(String::new(), |n| {
                                format!(", exposes {n} tool(s)")
                            });
                            (
                                Severity::Info,
                                format!(
                                    "probe: modern MCP — `{server}` supports protocol {versions_list} (server/discover){tools_note}"
                                ),
                            )
                        }
                        crate::probe::Probe::Reachable(reason) => {
                            (Severity::Info, format!("probe: {reason}"))
                        }
                        crate::probe::Probe::Unreachable(reason) => {
                            (Severity::Critical, format!("unreachable: {reason}"))
                        }
                        crate::probe::Probe::Skipped(reason) => {
                            (Severity::Info, format!("probe skipped: {reason}"))
                        }
                    };
                    doctor::Finding {
                        severity,
                        tool: *tool,
                        server: Some(name.clone()),
                        message,
                    }
                }))
            })
            .collect();
        handles
            .into_iter()
            .map(|(tool, name, h)| match h.join() {
                Ok(finding) => finding,
                Err(_) => doctor::Finding {
                    severity: Severity::Critical,
                    tool,
                    server: Some(name),
                    message: "unreachable: probe thread panicked".to_owned(),
                },
            })
            .collect::<Vec<_>>()
    })
}
