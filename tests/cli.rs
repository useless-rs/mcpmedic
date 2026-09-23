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
