//! End-to-end tests: run the compiled binary against a throwaway home
//! directory full of realistic configs.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn temp_home(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "mcpmedic-e2e-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mcpmedic"))
        .args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("NO_COLOR", "1")
        .env_remove("CODEX_HOME")
        .env_remove("CLAUDE_CONFIG_DIR")
        .output()
        .expect("failed to run mcpmedic")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

fn write(home: &Path, rel: &str, contents: &str) {
    let path = home.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn ubiquitous_command() -> &'static str {
    if cfg!(windows) { "cmd" } else { "sh" }
}

fn sample_home(tag: &str) -> PathBuf {
    let home = temp_home(tag);
    let cmd = ubiquitous_command();
    write(
        &home,
        ".cursor/mcp.json",
        &format!(
            r#"{{
  "mcpServers": {{
    "context7": {{"command": "{cmd}", "args": ["-y", "@upstash/context7-mcp"]}},
    "github": {{"type": "http", "url": "https://api.githubcopilot.com/mcp/"}}
  }}
}}"#
        ),
    );
    write(
        &home,
        ".claude.json",
        &format!(
            r#"{{
  "numStartups": 42,
  "mcpServers": {{
    "context7": {{"command": "{cmd}", "args": ["-y", "@upstash/context7-mcp-old"]}},
    "only-claude": {{"command": "{cmd}", "args": ["mcp-server-fetch"]}}
  }}
}}"#
        ),
    );
    home
}

#[test]
fn list_shows_servers_from_both_tools() {
    let home = sample_home("list");
    let out = run(&home, &["list"]);
    assert!(out.status.success(), "stdout: {}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("cursor"));
    assert!(text.contains("context7"));
    assert!(text.contains("github"));
    assert!(text.contains("only-claude"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn scan_counts_tools_and_servers() {
    let home = sample_home("scan");
    let out = run(&home, &["scan"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("2 tool(s) configured"));
    assert!(text.contains("4 servers total"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn show_finds_server_across_tools_and_reports_drift() {
    let home = sample_home("show");
    let out = run(&home, &["show", "context7"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("configured in 2 tool(s)"), "got: {text}");
    assert!(text.contains("drift"), "expected drift warning in: {text}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn show_unknown_server_fails() {
    let home = sample_home("show-miss");
    let out = run(&home, &["show", "nope"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("no tool configures a server named `nope`"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn doctor_exits_one_on_dead_command_and_drift() {
    let home = sample_home("doctor");
    write(
        &home,
        ".cursor/mcp.json",
        r#"{"mcpServers": {"dead": {"command": "/no/such/binary", "args": []}}}"#,
    );
    let out = run(&home, &["doctor"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(text.contains("1 critical"), "got: {text}");
    assert!(text.contains("was not found on this machine"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn doctor_flags_broken_json_and_reports_parse_error() {
    let home = temp_home("doctor-parse");
    write(&home, ".cursor/mcp.json", "{ this is not json");
    let out = run(&home, &["doctor"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stdout(&out).contains("cannot be parsed"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn healthy_configs_doctor_cleanly() {
    let home = sample_home("doctor-clean");
    let out = run(&home, &["doctor"]);
    // context7 drift exists in the sample, so this is a warning, not critical.
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("warning"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn diff_reports_only_in_and_drifted() {
    let home = sample_home("diff");
    let out = run(&home, &["diff", "cursor", "claude-code"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("only in cursor"), "got: {text}");
    assert!(text.contains("only in claude-code"), "got: {text}");
    assert!(text.contains("context7 —"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn add_persists_snippet_and_creates_backup() {
    let home = sample_home("add");
    let out = run(
        &home,
        &[
            "add",
            "playwright",
            "--to",
            "cursor",
            "--command",
            "npx",
            "--env",
            "HEADLESS=1",
            "--",
            "-y",
            "@playwright/mcp",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(config["mcpServers"]["playwright"]["command"], "npx");
    assert_eq!(config["mcpServers"]["playwright"]["env"]["HEADLESS"], "1");
    // pre-existing servers untouched
    assert!(config["mcpServers"]["context7"].is_object());

    let backups = std::fs::read_dir(home.join(".mcpmedic/backups"))
        .unwrap()
        .count();
    assert_eq!(backups, 1, "expected one automatic backup");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn add_dry_run_touches_nothing() {
    let home = sample_home("add-dry");
    let before = std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap();
    let out = run(
        &home,
        &[
            "add",
            "x",
            "--to",
            "cursor",
            "--command",
            "npx",
            "--dry-run",
        ],
    );
    assert!(out.status.success());
    let after = std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap();
    assert_eq!(before, after);
    assert!(!home.join(".mcpmedic").exists());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn add_to_missing_tool_without_directory_fails() {
    let home = temp_home("add-missing");
    let out = run(&home, &["add", "x", "--to", "zed", "--command", "npx"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("is the tool installed?"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn add_duplicate_is_rejected() {
    let home = sample_home("add-dup");
    let out = run(
        &home,
        &[
            "add",
            "github",
            "--to",
            "cursor",
            "--url",
            "https://example.com",
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("already exists"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn add_remote_url_to_vscode_emits_type_field() {
    let home = temp_home("add-vscode");
    write(&home, ".vscode/mcp.json", "{}");
    let out = run(
        &home,
        &[
            "add",
            "docs",
            "--to",
            "vscode",
            "--url",
            "https://docs.example.com/mcp",
            "--header",
            "Authorization=Bearer tok",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".vscode/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(config["servers"]["docs"]["type"], "http");
    assert_eq!(
        config["servers"]["docs"]["url"],
        "https://docs.example.com/mcp"
    );
    assert_eq!(
        config["servers"]["docs"]["headers"]["Authorization"],
        "Bearer tok"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn rm_removes_only_the_named_server() {
    let home = sample_home("rm");
    let out = run(&home, &["rm", "github", "--from", "cursor"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert!(config["mcpServers"].get("github").is_none());
    assert!(config["mcpServers"]["context7"].is_object());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn rm_unknown_server_fails() {
    let home = sample_home("rm-miss");
    let out = run(&home, &["rm", "ghost", "--from", "cursor"]);
    assert_eq!(out.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn sync_copies_missing_servers_additively() {
    let home = sample_home("sync");
    let out = run(
        &home,
        &[
            "sync",
            "--from",
            "cursor",
            "--to",
            "claude-code",
            "--names",
            "github,ghost",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("github (added"), "got: {text}");
    assert!(text.contains("not found in source: ghost"), "got: {text}");

    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(config["numStartups"], 42, "unrelated keys must survive");
    assert_eq!(
        config["mcpServers"]["github"]["url"],
        "https://api.githubcopilot.com/mcp/"
    );
    // target-only servers are never deleted
    assert!(config["mcpServers"]["only-claude"].is_object());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn sync_force_overwrites_drifted_server() {
    let home = sample_home("sync-force");
    let out = run(
        &home,
        &[
            "sync",
            "--from",
            "cursor",
            "--to",
            "claude-code",
            "--names",
            "context7",
            "--force",
        ],
    );
    assert!(out.status.success());
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(
        config["mcpServers"]["context7"]["args"][1],
        "@upstash/context7-mcp"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn sync_without_force_skips_drift() {
    let home = sample_home("sync-skip");
    let out = run(
        &home,
        &[
            "sync",
            "--from",
            "cursor",
            "--to",
            "claude-code",
            "--names",
            "context7",
        ],
    );
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(text.contains("drifted — skipped"), "got: {text}");
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(
        config["mcpServers"]["context7"]["args"][1], "@upstash/context7-mcp-old",
        "target must not change without --force"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn export_and_import_roundtrip_restores_a_wiped_config() {
    let home = sample_home("roundtrip");
    let export_file = home.join("export.json");

    let out = run(&home, &["export", "--out", export_file.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    std::fs::remove_file(home.join(".cursor/mcp.json")).unwrap();
    write(&home, ".cursor/mcp.json", "{}");

    let out = run(&home, &["import", export_file.to_str().unwrap()]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(
        config["mcpServers"]["context7"]["command"],
        serde_json::json!(ubiquitous_command())
    );
    assert_eq!(
        config["mcpServers"]["github"]["url"],
        "https://api.githubcopilot.com/mcp/"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn codex_toml_edits_preserve_comments_and_other_settings() {
    let home = temp_home("codex");
    write(
        &home,
        ".codex/config.toml",
        r#"# main profile
model = "gpt-5.2"

[mcp_servers.keep]
command = "node"
args = ["one.js"]
"#,
    );

    let out = run(
        &home,
        &[
            "add",
            "fresh",
            "--to",
            "codex",
            "--command",
            "npx",
            "--",
            "-y",
            "pkg",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let out = run(&home, &["rm", "keep", "--from", "codex"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let toml = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(toml.contains("# main profile"));
    assert!(toml.contains("model = \"gpt-5.2\""));
    assert!(toml.contains("[mcp_servers.fresh]"));
    assert!(!toml.contains("[mcp_servers.keep]"));

    let listing = run(&home, &["list", "--tool", "codex"]);
    assert!(stdout(&listing).contains("fresh"));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn opencode_jsonc_is_read_but_never_edited() {
    let home = temp_home("opencode");
    write(
        &home,
        ".config/opencode/opencode.json",
        "{\n  // my servers\n  \"mcp\": {\n    \"ctx7\": {\"type\": \"local\", \"command\": [\"bun\", \"x\", \"c7\"]}\n  }\n}",
    );

    let list = run(&home, &["list", "--tool", "opencode"]);
    assert!(list.status.success());
    assert!(stdout(&list).contains("ctx7"), "got: {}", stdout(&list));

    let out = run(&home, &["rm", "ctx7", "--from", "opencode"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("read-only"), "got: {}", stderr(&out));

    let raw = std::fs::read_to_string(home.join(".config/opencode/opencode.json")).unwrap();
    assert!(raw.contains("// my servers"), "file must be untouched");
    let _ = std::fs::remove_dir_all(&home);
}

fn zed_settings_rel() -> &'static str {
    if cfg!(windows) {
        "AppData/Roaming/Zed/settings.json"
    } else {
        ".config/zed/settings.json"
    }
}

#[test]
fn zed_context_servers_are_edited_without_touching_other_settings() {
    let home = temp_home("zed");
    write(
        &home,
        zed_settings_rel(),
        r#"{"theme": "One Dark", "context_servers": {"ctx7": {"command": "npx", "args": ["-y", "@upstash/context7-mcp"]}}}"#,
    );
    let out = run(
        &home,
        &[
            "add",
            "extra",
            "--to",
            "zed",
            "--url",
            "https://example.com/mcp",
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(zed_settings_rel())).unwrap())
            .unwrap();
    assert_eq!(config["theme"], "One Dark");
    assert_eq!(
        config["context_servers"]["extra"]["url"],
        "https://example.com/mcp"
    );
    assert!(config["context_servers"]["ctx7"].is_object());
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn doctor_fix_repairs_configs_and_keeps_backups() {
    let home = temp_home("doctor-fix");
    let cmd = ubiquitous_command();
    write(
        &home,
        ".vscode/mcp.json",
        &format!(r#"{{"servers": {{"nope": {{"command": "{cmd}"}}}}}}"#),
    );
    write(
        &home,
        zed_settings_rel(),
        &format!(
            r#"{{"theme": "One Dark", "context_servers": {{"old": {{"command": {{"path": "{cmd}", "args": ["-c"]}}}}}}}}"#
        ),
    );

    let out = run(&home, &["doctor"]);
    assert_eq!(out.status.code(), Some(1), "missing type is critical");

    let out = run(&home, &["doctor", "--fix"]);
    assert!(
        out.status.success(),
        "stderr: {}|stdout: {}",
        stderr(&out),
        stdout(&out)
    );
    let text = stdout(&out);
    assert!(text.contains("added missing `type: stdio`"), "got: {text}");
    assert!(text.contains("legacy nested `command`"), "got: {text}");
    assert!(text.contains("everything looks healthy"), "got: {text}");

    let vscode: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".vscode/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(vscode["servers"]["nope"]["type"], "stdio");

    let zed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(zed_settings_rel())).unwrap())
            .unwrap();
    assert_eq!(zed["theme"], "One Dark");
    assert_eq!(zed["context_servers"]["old"]["command"], cmd);

    let backups = std::fs::read_dir(home.join(".mcpmedic/backups"))
        .unwrap()
        .count();
    assert_eq!(backups, 2, "one backup per repaired config");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn doctor_fix_dry_run_touches_nothing() {
    let home = temp_home("doctor-fix-dry");
    let cmd = ubiquitous_command();
    write(
        &home,
        ".vscode/mcp.json",
        &format!(r#"{{"servers": {{"nope": {{"command": "{cmd}"}}}}}}"#),
    );
    let before = std::fs::read_to_string(home.join(".vscode/mcp.json")).unwrap();

    let out = run(&home, &["doctor", "--fix", "--dry-run"]);
    assert!(stdout(&out).contains("would fix"), "got: {}", stdout(&out));
    assert_eq!(
        out.status.code(),
        Some(1),
        "unfixed findings still fail the check"
    );
    assert_eq!(
        before,
        std::fs::read_to_string(home.join(".vscode/mcp.json")).unwrap()
    );
    assert!(!home.join(".mcpmedic").exists(), "no backups on dry run");

    let out = run(&home, &["doctor", "--dry-run"]);
    assert_eq!(out.status.code(), Some(2), "--dry-run requires --fix");
    let _ = std::fs::remove_dir_all(&home);
}

fn windsurf_config_rel() -> &'static str {
    if cfg!(windows) {
        "AppData/Roaming/Codeium/windsurf/mcp_config.json"
    } else {
        ".codeium/windsurf/mcp_config.json"
    }
}

#[test]
fn add_remote_uses_and_reads_back_each_tools_dialect() {
    let home = temp_home("dialects");
    write(&home, ".cursor/mcp.json", "{}");
    write(&home, ".gemini/settings.json", "{}");
    write(&home, windsurf_config_rel(), "{}");

    for tool in ["cursor", "gemini-cli", "windsurf"] {
        let out = run(
            &home,
            &[
                "add",
                "docs",
                "--to",
                tool,
                "--url",
                "https://example.com/mcp",
            ],
        );
        assert!(out.status.success(), "{tool}: {}", stderr(&out));
    }

    let cursor: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(
        cursor["mcpServers"]["docs"]["url"],
        "https://example.com/mcp"
    );
    assert!(
        cursor["mcpServers"]["docs"].get("type").is_none(),
        "cursor infers the transport — no type field"
    );

    let gemini: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".gemini/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        gemini["mcpServers"]["docs"]["httpUrl"],
        "https://example.com/mcp"
    );
    assert!(gemini["mcpServers"]["docs"].get("url").is_none());

    let windsurf: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(windsurf_config_rel())).unwrap())
            .unwrap();
    assert_eq!(
        windsurf["mcpServers"]["docs"]["serverUrl"],
        "https://example.com/mcp"
    );
    assert!(windsurf["mcpServers"]["docs"].get("type").is_none());

    for tool in ["cursor", "gemini-cli", "windsurf"] {
        let out = run(&home, &["list", "--tool", tool]);
        assert!(
            stdout(&out).contains("https://example.com/mcp"),
            "every dialect must read back as the same normalized server ({tool}): {}",
            stdout(&out)
        );
    }

    let out = run(&home, &["diff", "cursor", "gemini-cli"]);
    assert!(
        stdout(&out).contains("no drift"),
        "spelling differences must not count as drift: {}",
        stdout(&out)
    );

    let out = run(&home, &["doctor"]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "doctor must stay quiet on entries mcpmedic itself wrote: {}",
        stdout(&out)
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn disable_parks_servers_and_doctor_skips_them() {
    let home = temp_home("disable");
    let cmd = ubiquitous_command();
    write(
        &home,
        zed_settings_rel(),
        &format!(
            r#"{{"theme": "One Dark", "context_servers": {{"parked": {{"command": "/no/such/binary", "enabled": false}}, "live": {{"command": "{cmd}"}}}}}}"#
        ),
    );

    // A parked server with a dead command is intentional, not a finding.
    let out = run(&home, &["doctor"]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));

    // Parked rows are marked in listings.
    let out = run(&home, &["list", "--tool", "zed"]);
    assert!(
        stdout(&out).contains("(off)"),
        "parked rows are marked: {}",
        stdout(&out)
    );

    // Park the live one via mcpmedic: config kept, flag written.
    let out = run(&home, &["disable", "live", "--from", "zed"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let zed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(zed_settings_rel())).unwrap())
            .unwrap();
    assert_eq!(zed["theme"], "One Dark", "unrelated keys must survive");
    assert_eq!(zed["context_servers"]["live"]["enabled"], false);
    assert!(
        zed["context_servers"]["live"]["command"].is_string(),
        "the config is kept, not removed"
    );

    // Resume the parked one; its dead command becomes a finding again.
    let out = run(&home, &["enable", "parked", "--from", "zed"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let zed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(zed_settings_rel())).unwrap())
            .unwrap();
    assert_eq!(zed["context_servers"]["parked"]["enabled"], true);
    let out = run(&home, &["doctor"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "the resumed dead command must be flagged again: {}",
        stdout(&out)
    );

    let backups = std::fs::read_dir(home.join(".mcpmedic/backups"))
        .unwrap()
        .count();
    assert_eq!(backups, 2, "one automatic backup per mutation");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn disable_parks_codex_servers_preserving_toml_comments() {
    let home = temp_home("disable-codex");
    write(
        &home,
        ".codex/config.toml",
        "# profile comment\nmodel = \"gpt-5.2\"\n\n[mcp_servers.svc]\ncommand = \"node\"\n",
    );
    let out = run(&home, &["disable", "svc", "--from", "codex"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let toml = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(toml.contains("# profile comment"));
    assert!(toml.contains("enabled = false"));
    assert!(toml.contains("[mcp_servers.svc]"));
    assert!(toml.contains("command = \"node\""));

    let out = run(&home, &["enable", "svc", "--from", "codex"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(
        std::fs::read_to_string(home.join(".codex/config.toml"))
            .unwrap()
            .contains("enabled = true")
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn disable_refuses_tools_without_a_documented_flag() {
    let home = sample_home("disable-refuse");
    let out = run(&home, &["disable", "context7", "--from", "cursor"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("no documented per-server disable switch"),
        "got: {}",
        stderr(&out)
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn project_mode_reads_and_writes_project_configs() {
    let home = temp_home("project-mode");
    let project = home.join("repo");
    let cmd = ubiquitous_command();
    write(
        &project,
        ".mcp.json",
        &format!(r#"{{"mcpServers": {{"shared": {{"command": "{cmd}"}}}}}}"#),
    );
    write(
        &project,
        ".cursor/mcp.json",
        &format!(r#"{{"mcpServers": {{"shared": {{"command": "{cmd}"}}}}}}"#),
    );
    let project_arg = project.to_str().unwrap();

    // Scan in project mode: only project-capable tools participate.
    let out = run(&home, &["scan", "--project", project_arg]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("claude-code"), "got: {text}");
    assert!(text.contains("cursor"), "got: {text}");
    assert!(!text.contains("zed"), "zed has no project scope: {text}");
    assert!(text.contains("2 tool(s) configured"), "got: {text}");

    // list shows project servers from both files
    let out = run(&home, &["list", "--project", project_arg]);
    assert!(stdout(&out).contains("shared"), "got: {}", stdout(&out));

    // diff: identical project configs across tools
    let out = run(
        &home,
        &["diff", "cursor", "claude-code", "--project", project_arg],
    );
    assert!(stdout(&out).contains("no drift"), "got: {}", stdout(&out));

    let out = run(
        &home,
        &[
            "add",
            "docs",
            "--to",
            "vscode",
            "--url",
            "https://example.com/mcp",
            "--project",
            project_arg,
        ],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let vscode: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(project.join(".vscode/mcp.json")).unwrap())
            .unwrap();
    assert_eq!(vscode["servers"]["docs"]["url"], "https://example.com/mcp");
    assert_eq!(vscode["servers"]["docs"]["type"], "http");

    // doctor in project mode checks the project files and stays healthy
    let out = run(&home, &["doctor", "--project", project_arg]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));

    // tools without a documented project scope refuse --project edits
    let out = run(
        &home,
        &[
            "add",
            "x",
            "--to",
            "zed",
            "--command",
            "sh",
            "--project",
            project_arg,
        ],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("no project-scoped config"),
        "got: {}",
        stderr(&out)
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn audit_flags_hardcoded_secrets_and_passes_clean_configs() {
    let home = temp_home("audit-dirty");
    write(
        &home,
        ".cursor/mcp.json",
        r#"{"mcpServers": {"leaky": {"command": "sh", "env": {"GITHUB_PAT": "ghp_0123456789abcdefghijklmnopqrstuvwxyzAB", "SAFE_KEY": "${FROM_SHELL}"}}}}"#,
    );
    let out = run(&home, &["audit"]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("GitHub token"), "got: {text}");
    assert!(text.contains("leaky"), "got: {text}");
    assert!(
        !text.contains("FROM_SHELL"),
        "templated values must not be flagged: {text}"
    );

    let clean = temp_home("audit-clean");
    write(
        &clean,
        ".cursor/mcp.json",
        r#"{"mcpServers": {"ok": {"command": "sh", "env": {"KEY": "${MY_KEY}"}}}}"#,
    );
    let out = run(&clean, &["audit"]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&clean);
}

#[test]
fn json_mode_emits_parseable_schema_versioned_output() {
    let home = sample_home("json");
    let out = run(&home, &["scan", "--json"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("scan --json must emit valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["tools"].is_array());
    assert!(parsed["summary"].is_object());

    let out = run(&home, &["list", "--json", "--tool", "cursor"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("list --json must emit valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["tools"][0]["servers"].is_array());
    assert!(parsed["tools"][0]["servers"][0]["name"].is_string());

    let out = run(&home, &["doctor", "--json"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("doctor --json must emit valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["findings"].is_array());
    assert!(parsed["summary"].is_object());

    let out = run(&home, &["audit", "--json"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("audit --json must emit valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["findings"].is_array());
    assert!(parsed["summary"].is_object());

    let out = run(&home, &["show", "context7", "--json"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("show --json must emit valid JSON");
    assert_eq!(parsed["schema_version"], 1);
    assert!(parsed["tools"].is_array());
    assert!(parsed["drift"].is_boolean());

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn warp_kiro_and_trae_are_discoverable() {
    let home = temp_home("new-tools");
    write(
        &home,
        ".warp/.mcp.json",
        r#"{"mcpServers": {"warp-srv": {"command": "sh"}}}"#,
    );
    write(
        &home,
        ".kiro/settings/mcp.json",
        r#"{"mcpServers": {"kiro-srv": {"command": "sh", "disabled": true}}}"#,
    );

    let out = run(&home, &["list", "--tool", "warp"]);
    assert!(stdout(&out).contains("warp-srv"), "{}", stdout(&out));

    let out = run(&home, &["list", "--tool", "kiro"]);
    assert!(stdout(&out).contains("kiro-srv"), "{}", stdout(&out));
    assert!(stdout(&out).contains("(off)"), "{}", stdout(&out));

    let out = run(&home, &["scan", "--json"]);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout(&out)).expect("scan --json must emit valid JSON");
    let ids: Vec<&str> = parsed["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(ids.contains(&"warp"), "warp must appear: {ids:?}");
    assert!(ids.contains(&"kiro"), "kiro must appear: {ids:?}");
    assert!(ids.contains(&"trae"), "trae must appear: {ids:?}");
    assert!(
        ids.contains(&"antigravity"),
        "antigravity must appear: {ids:?}"
    );
    assert!(ids.contains(&"copilot"), "copilot must appear: {ids:?}");
    assert!(ids.contains(&"kimi-code"), "kimi-code must appear: {ids:?}");
    assert!(ids.contains(&"qwen-code"), "qwen-code must appear: {ids:?}");
    assert!(ids.contains(&"auggie"), "auggie must appear: {ids:?}");
    assert!(
        ids.contains(&"factory-droid"),
        "factory-droid must appear: {ids:?}"
    );
    assert!(ids.contains(&"amp"), "amp must appear: {ids:?}");
    assert!(ids.contains(&"crush"), "crush must appear: {ids:?}");
    assert!(ids.contains(&"openhands"), "openhands must appear: {ids:?}");
    assert!(ids.contains(&"devin"), "devin must appear: {ids:?}");
    assert!(ids.contains(&"openclaw"), "openclaw must appear: {ids:?}");
    assert!(ids.contains(&"goose"), "goose must appear: {ids:?}");
    assert!(
        ids.contains(&"continue-yaml"),
        "continue-yaml must appear: {ids:?}"
    );
    assert_eq!(ids.len(), 29, "29 tools expected: {ids:?}");

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn restore_reverts_config_to_latest_backup() {
    let home = temp_home("restore");
    write(
        &home,
        ".cursor/mcp.json",
        r#"{"mcpServers": {"original": {"command": "sh"}}}"#,
    );

    let out = run(
        &home,
        &["add", "new-server", "--to", "cursor", "--command", "sh"],
    );
    assert!(out.status.success(), "stderr: {}", stderr(&out));

    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert!(config["mcpServers"]["new-server"].is_object());

    let out = run(&home, &["restore", "--latest", "--tool", "cursor"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("restored"), "{}", stdout(&out));

    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap())
            .unwrap();
    assert!(
        config["mcpServers"].get("new-server").is_none(),
        "restored config must not have the added server"
    );
    assert!(
        config["mcpServers"]["original"].is_object(),
        "original server must be back"
    );

    let out = run(&home, &["restore", "--list"]);
    assert!(stdout(&out).contains("cursor"), "{}", stdout(&out));

    let out = run(&home, &["restore"]);
    assert!(stdout(&out).contains("to restore"), "{}", stdout(&out));

    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn backup_command_copies_configs() {
    let home = sample_home("backup");
    let out = run(&home, &["backup"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("Claude Code"));
    assert!(stdout(&out).contains("Cursor"));
    let count = std::fs::read_dir(home.join(".mcpmedic/backups"))
        .unwrap()
        .count();
    assert_eq!(count, 2);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn completions_are_generated_for_every_shell() {
    let home = temp_home("completions");
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let out = run(&home, &["completions", shell]);
        assert!(out.status.success(), "{shell}: stderr: {}", stderr(&out));
        let text = stdout(&out);
        assert!(
            text.contains("mcpmedic"),
            "{shell} script must name the binary"
        );
        assert!(!text.is_empty(), "{shell} script must not be empty");
    }

    let out = run(&home, &["completions", "bash"]);
    assert!(
        stdout(&out).contains("complete"),
        "bash script should register a complete handler"
    );

    let out = run(&home, &["completions", "not-a-shell"]);
    assert_eq!(out.status.code(), Some(2));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn init_reports_when_nothing_found() {
    let home = temp_home("init-empty");
    let out = run(&home, &["init"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("No MCP servers found"), "got: {stdout}");
}

#[test]
fn init_picks_richest_and_syncs_to_installed_tools() {
    let home = temp_home("init-sync");
    std::fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"alpha":{"command":"echo","args":["a"]},"beta":{"command":"echo","args":["b"]}}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    let out = run(&home, &["init"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("claude-code"), "source reported: {stdout}");
    let cursor: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".cursor").join("mcp.json")).unwrap(),
    )
    .unwrap();
    let servers = cursor["mcpServers"].as_object().unwrap();
    assert!(servers.contains_key("alpha"), "alpha synced: {cursor}");
    assert!(servers.contains_key("beta"), "beta synced: {cursor}");
}

#[test]
fn init_json_reports_source_without_touching_files() {
    let home = temp_home("init-json");
    std::fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"alpha":{"command":"echo","args":["a"]}}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    let out = run(&home, &["--json", "init"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["source"], "claude-code");
    assert!(
        !home.join(".cursor").join("mcp.json").exists(),
        "json mode must not touch files"
    );
}

#[test]
fn doctor_probe_reports_mcp_handshake() {
    let home = temp_home("probe-handshake");
    let legacy_err =
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}"#;
    let init = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"mock-mcp","version":"1.0"}}}"#;
    let tools = r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"a"},{"name":"b"}]}}"#;
    let script = format!(
        "read a; printf '%s\\n' '{legacy_err}'; read b; printf '%s\\n' '{init}'; read c; read d; printf '%s\\n' '{tools}'"
    );
    let discover_result = r#"{"jsonrpc":"2.0","id":1,"result":{"supportedVersions":["2026-07-28"],"capabilities":{"tools":{}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"modern-srv","version":"1.0"}}}}"#;
    let modern_tools = r#"{"jsonrpc":"2.0","id":2,"result":{"resultType":"complete","tools":[{"name":"x"},{"name":"y"},{"name":"z"}]}}"#;
    let modern_script = format!(
        "read a; printf '%s\\n' '{discover_result}'; read b; printf '%s\\n' '{modern_tools}'"
    );
    let cfg = format!(
        r#"{{"mcpServers":{{"mock":{{"command":"sh","args":["-c",{}]}},"modern":{{"command":"sh","args":["-c",{}]}},"broken":{{"command":"definitely-missing-cmd-xyz","args":[]}},"noisy":{{"command":"sh","args":["-c","echo boom detail >&2; exit 1"]}}}}}}"#,
        serde_json::to_string(&script).unwrap(),
        serde_json::to_string(&modern_script).unwrap()
    );
    std::fs::write(home.join(".claude.json"), cfg).unwrap();
    let out = run(&home, &["--json", "doctor", "--probe"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("MCP handshake ok") && stdout.contains("mock-mcp"),
        "handshake result missing: {stdout}"
    );
    assert!(
        stdout.contains("exposes 2 tool"),
        "tool count missing: {stdout}"
    );
    assert!(
        stdout.contains("modern MCP"),
        "modern discover missing: {stdout}"
    );
    assert!(
        stdout.contains("modern-srv"),
        "modern server missing: {stdout}"
    );
    assert!(
        stdout.contains("exposes 3 tool"),
        "modern tool count missing: {stdout}"
    );
    assert!(
        stdout.contains("definitely-missing-cmd-xyz") || stdout.contains("unreachable"),
        "unreachable finding missing: {stdout}"
    );
    assert!(
        stdout.contains("boom detail"),
        "stderr tail missing: {stdout}"
    );
}

#[test]
fn doctor_flags_npx_footguns() {
    let home = temp_home("npx-footgun");
    std::fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"mem":{"command":"npx","args":["@modelcontextprotocol/server-memory@latest"]}}}"#,
    )
    .unwrap();
    let out = run(&home, &["doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("npx without -y"), "{stdout}");
    assert!(stdout.contains("@latest forces"), "{stdout}");
}

#[test]
fn preset_lists_bundles() {
    let home = temp_home("preset-list");
    let out = run(&home, &["preset", "list"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("minimal"), "{stdout}");
    assert!(stdout.contains("demo"), "{stdout}");
    assert!(stdout.contains("server-memory"), "{stdout}");
}

#[test]
fn preset_add_writes_bundle_to_tool() {
    let home = temp_home("preset-add");
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    let out = run(&home, &["preset", "add", "minimal", "--to", "cursor"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("memory"), "{stdout}");
    assert!(stdout.contains("sequential-thinking"), "{stdout}");
    let cfg: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(home.join(".cursor").join("mcp.json")).unwrap(),
    )
    .unwrap();
    let servers = cfg["mcpServers"].as_object().unwrap();
    assert!(servers.contains_key("memory"), "{cfg}");
    assert!(servers.contains_key("sequential-thinking"), "{cfg}");
    assert_eq!(
        servers["memory"]["args"][1],
        "@modelcontextprotocol/server-memory"
    );
}

#[test]
fn preset_add_skips_existing_and_unknown_fails() {
    let home = temp_home("preset-skip");
    std::fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"memory":{"command":"my-custom-memory","args":[]}}}"#,
    )
    .unwrap();
    let out = run(&home, &["preset", "add", "minimal", "--to", "claude-code"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("skipped"), "{stdout}");
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(cfg["mcpServers"]["memory"]["command"], "my-custom-memory");
    assert!(
        cfg["mcpServers"]
            .as_object()
            .unwrap()
            .contains_key("sequential-thinking")
    );

    let out = run(&home, &["preset", "add", "nope", "--to", "claude-code"]);
    assert!(!out.status.success());
}

#[test]
fn doctor_fix_inserts_npx_yes_flag() {
    let home = temp_home("fix-npx-yes");
    std::fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"mem":{"command":"npx","args":["@modelcontextprotocol/server-memory"]}}}"#,
    )
    .unwrap();

    let out = run(&home, &["doctor", "--fix", "--dry-run"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("would fix"), "{stdout}");
    assert!(stdout.contains("-y"), "{stdout}");
    let raw = std::fs::read_to_string(home.join(".claude.json")).unwrap();
    assert!(!raw.contains("\"-y\""), "dry-run must not touch the file");

    let out = run(&home, &["doctor", "--fix"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("added `-y` to npx"), "{stdout}");
    let cfg: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude.json")).unwrap()).unwrap();
    assert_eq!(cfg["mcpServers"]["mem"]["args"][0], "-y");
    assert_eq!(
        cfg["mcpServers"]["mem"]["args"][1],
        "@modelcontextprotocol/server-memory"
    );
}

#[test]
fn doctor_explain_prints_fix_hints() {
    let home = temp_home("doctor-explain");
    std::fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"mem":{"command":"npx","args":["@modelcontextprotocol/server-memory"]}}}"#,
    )
    .unwrap();
    let out = run(&home, &["doctor", "--explain"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("How to fix"), "{stdout}");
    assert!(stdout.contains("doctor --fix"), "{stdout}");
    let out = run(&home, &["--json", "doctor"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("\"fix\":"), "{stdout}");
    assert!(stdout.contains("doctor --fix"), "{stdout}");
}

#[test]
fn openclaw_nested_servers_are_read() {
    let home = temp_home("openclaw");
    std::fs::create_dir_all(home.join(".openclaw")).unwrap();
    std::fs::write(
        home.join(".openclaw").join("openclaw.json"),
        r#"{
            "mcp": {
                "servers": {
                    "docs": {"command": "uvx", "args": ["mcp-server-fetch"]}
                }
            }
        }"#,
    )
    .unwrap();
    let out = run(&home, &["--json", "list", "--tool", "openclaw"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("docs"), "{stdout}");
    assert!(stdout.contains("openclaw"), "{stdout}");
}

#[test]
fn doctor_probe_skips_parked_servers() {
    let home = temp_home("probe-parked");
    std::fs::create_dir_all(home.join(".kiro").join("settings")).unwrap();
    std::fs::write(
        home.join(".kiro").join("settings").join("mcp.json"),
        r#"{"mcpServers":{"parked":{"command":"definitely-missing-cmd-xyz","args":[],"disabled":true}}}"#,
    )
    .unwrap();
    let out = run(&home, &["--json", "doctor", "--probe", "--tool", "kiro"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("definitely-missing-cmd-xyz"),
        "parked server must not be probed: {stdout}"
    );
    assert!(
        !stdout.contains("unreachable"),
        "parked server must not produce probe findings: {stdout}"
    );
}

#[test]
fn diff_json_reports_drift_buckets() {
    let home = temp_home("diff-json");
    std::fs::write(
        home.join(".claude.json"),
        r#"{"mcpServers":{"alpha":{"command":"echo","args":["a"]},"shared":{"command":"echo","args":["s"]}}}"#,
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    std::fs::write(
        home.join(".cursor").join("mcp.json"),
        r#"{"mcpServers":{"beta":{"command":"echo","args":["b"]},"shared":{"command":"echo","args":["different"]}}}"#,
    )
    .unwrap();
    let out = run(&home, &["--json", "diff", "claude-code", "cursor"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["a"], "claude-code");
    assert_eq!(v["b"], "cursor");
    assert!(
        v["only_a"].as_array().unwrap().iter().any(|s| s == "alpha"),
        "{stdout}"
    );
    assert!(
        v["only_b"].as_array().unwrap().iter().any(|s| s == "beta"),
        "{stdout}"
    );
    assert_eq!(v["identical"], 0);
    assert_eq!(v["in_sync"], false);
    let different = v["different"].as_array().unwrap();
    assert_eq!(different.len(), 1);
    assert_eq!(different[0]["name"], "shared");
}

#[test]
fn doctor_probe_timeout_extends_the_budget() {
    let home = temp_home("probe-timeout");
    let resp = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{},"serverInfo":{"name":"slow-srv","version":"1.0"}}}"#;
    let script = format!("read a; sleep 4.5; printf '%s\\n' '{resp}'; read b; read c");
    std::fs::write(
        home.join(".claude.json"),
        format!(
            r#"{{"mcpServers":{{"slow":{{"command":"sh","args":["-c",{}]}}}}}}"#,
            serde_json::to_string(&script).unwrap()
        ),
    )
    .unwrap();
    let out = run(
        &home,
        &["--json", "doctor", "--probe", "--tool", "claude-code"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("slow startup is common"),
        "default budget should miss: {stdout}"
    );
    let out = run(
        &home,
        &[
            "--json",
            "doctor",
            "--probe",
            "--probe-timeout",
            "10000",
            "--tool",
            "claude-code",
        ],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("MCP handshake ok"),
        "raised budget should land: {stdout}"
    );
    assert!(stdout.contains("slow-srv"), "{stdout}");
}

#[test]
fn yaml_tools_are_read_end_to_end() {
    let home = temp_home("yaml-tools");
    std::fs::create_dir_all(home.join(".config").join("goose")).unwrap();
    std::fs::write(
        home.join(".config").join("goose").join("config.yaml"),
        "provider: anthropic\nextensions:\n  - name: fs\n    enabled: false\n    transport:\n      type: stdio\n      command: npx\n      args: [\"-y\", \"pkg\"]\n",
    )
    .unwrap();
    std::fs::create_dir_all(home.join(".continue")).unwrap();
    std::fs::write(
        home.join(".continue").join("config.yaml"),
        "name: My Config\nmcpServers:\n  - name: sqlite\n    command: uvx\n    args:\n      - mcp-server-sqlite\n",
    )
    .unwrap();
    let out = run(&home, &["--json", "list", "--tool", "goose"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let servers = v["tools"][0]["servers"].as_array().unwrap();
    assert_eq!(servers[0]["name"], "fs");
    assert_eq!(servers[0]["disabled"], true);
    assert_eq!(servers[0]["transport"], "stdio");
    let out = run(&home, &["--json", "list", "--tool", "continue-yaml"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let servers = v["tools"][0]["servers"].as_array().unwrap();
    assert_eq!(servers[0]["name"], "sqlite");
    assert_eq!(servers[0]["transport"], "stdio");
}

#[test]
fn edit_print_reports_config_path() {
    let home = temp_home("edit-print");
    write(
        &home,
        ".claude.json",
        r#"{"mcpServers": {"x": {"command": "sh"}}}"#,
    );
    let out = run(&home, &["edit", "claude-code", "--print"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains(".claude.json"));
}

#[test]
fn edit_missing_config_fails() {
    let home = temp_home("edit-missing");
    let out = run(&home, &["edit", "claude-code", "--print"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("no config found"));
}

#[test]
fn edit_unknown_tool_fails() {
    let home = temp_home("edit-unknown");
    let out = run(&home, &["edit", "nosuchtool", "--print"]);
    assert!(!out.status.success());
    assert!(stderr(&out).contains("unknown tool"));
}

#[test]
fn edit_broken_editor_fails() {
    let home = temp_home("edit-broken-editor");
    write(
        &home,
        ".claude.json",
        r#"{"mcpServers": {"x": {"command": "sh"}}}"#,
    );
    let out = Command::new(env!("CARGO_BIN_EXE_mcpmedic"))
        .args(["edit", "claude-code"])
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("NO_COLOR", "1")
        .env("VISUAL", "definitely-not-an-editor-xyz")
        .env("EDITOR", "definitely-not-an-editor-xyz")
        .output()
        .expect("failed to run mcpmedic");
    assert!(!out.status.success());
    assert!(stderr(&out).contains("failed to launch editor"));
}

#[test]
fn scan_quiet_suppresses_brand_and_hints() {
    let home = sample_home("scan-quiet");
    let out = run(&home, &["scan", "--quiet"]);
    assert!(out.status.success());
    let text = stdout(&out);
    assert!(!text.contains("first aid"), "brand leaked: {text}");
    assert!(!text.contains("next:"), "hint leaked: {text}");
    assert!(text.contains("cursor"), "rows missing: {text}");
    assert!(
        text.contains("2 tool(s) configured"),
        "summary missing: {text}"
    );
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn diff_exit_code_gates_on_drift() {
    let home = sample_home("diff-exit");
    let plain = run(&home, &["diff", "cursor", "claude-code"]);
    assert!(plain.status.success(), "default must stay 0");
    let gated = run(&home, &["diff", "cursor", "claude-code", "--exit-code"]);
    assert_eq!(gated.status.code(), Some(1), "drift must exit 1");
    let same = run(&home, &["diff", "cursor", "cursor", "--exit-code"]);
    assert!(same.status.success(), "in-sync must stay 0");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn sync_all_reports_unwritable_targets() {
    let home = sample_home("sync-all-skip");
    write(&home, ".codeium/windsurf/mcp_config.json", "{broken json");
    let out = run(&home, &["sync-all", "--from", "cursor", "--dry-run"]);
    assert!(out.status.success(), "stderr: {}", stderr(&out));
    assert!(stdout(&out).contains("skipped"), "got: {}", stdout(&out));
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn restore_dry_run_touches_nothing() {
    let home = sample_home("restore-dry");
    let added = run(
        &home,
        &["add", "tmp-srv", "--to", "cursor", "--command", "sh"],
    );
    assert!(added.status.success(), "stderr: {}", stderr(&added));
    let before = std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap();
    let plan = run(&home, &["restore", "--latest", "--dry-run"]);
    assert!(plan.status.success(), "stderr: {}", stderr(&plan));
    assert!(
        stdout(&plan).contains("would restore"),
        "got: {}",
        stdout(&plan)
    );
    let after = std::fs::read_to_string(home.join(".cursor/mcp.json")).unwrap();
    assert_eq!(before, after, "dry run must not write");
    assert!(after.contains("tmp-srv"), "added server must survive plan");
    let _ = std::fs::remove_dir_all(&home);
}
