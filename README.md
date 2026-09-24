<div align="center">

<img src="https://raw.githubusercontent.com/useless-rs/mcpmedic/main/docs/brand/banner.png" width="880" alt="mcpmedic — first aid for MCP configs: scan, doctor, diff and sync MCP servers across every AI tool you use">

[![CI](https://github.com/useless-rs/mcpmedic/actions/workflows/ci.yml/badge.svg)](https://github.com/useless-rs/mcpmedic/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/mcpmedic.svg)](https://crates.io/crates/mcpmedic)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Made with Rust](https://img.shields.io/badge/made%20with-Rust-orange.svg)](https://www.rust-lang.org)

**One binary that reads, health-checks, diffs and syncs MCP servers across
every AI tool you use** — Claude Code, Cursor, Windsurf, VS Code, Zed, Codex,
Gemini CLI, Cline, Roo Code, Claude Desktop and opencode.

</div>

---

## The problem

If you use more than one AI coding tool in 2026, you already live it:

- **Claude Code** keeps servers in `~/.claude.json` under `mcpServers`
- **Cursor** uses `~/.cursor/mcp.json`
- **VS Code** wants a `servers` key where every entry *must* have a `type` field
- **Zed** nests everything under `context_servers`
- **Codex CLI** abandoned JSON entirely for `[mcp_servers]` TOML tables
- **Windsurf**, **Gemini CLI**, **Cline**, **Roo Code**, **Claude Desktop** — each
  with its own file in its own corner of your home directory

So the same one-sentence fact — *"there is a Supabase MCP server at this URL"* —
is hand-copied into five files in five dialects. They immediately start to
drift. You update a token in one tool and forget the other four. A dead
`command:` path breaks a tool silently. Nobody can answer "what MCP servers am I
running, and where?" without opening half a dozen config files.

**mcpmedic** is the missing `package.json`-style tooling for that mess: it
normalizes every dialect into one model, health-checks it, shows the drift
between tools, and edits configs **surgically** — never touching a key it
doesn't own, always atomically, always with an automatic backup first.

## Highlights

- 🚀 **`init`** — one-shot onboarding: detect your configs, sync the richest everywhere
- 📦 **`preset`** — curated zero-config bundles (`minimal`, `demo`): one command to a working setup
- 🩺 **`doctor`** — parse errors, dead `command:` paths, unset `${VAR}` env references, npx footguns (`npx` missing `-y`, `@latest` round-trips), VS Code entries
  missing their mandatory `type`, remote entries whose `type` spelling their
  tool can't read, cross-tool config drift, duplicate servers, and
  context-window bloat warnings — and **`--fix`** applies the provably safe
  repairs (VS Code `type`, Zed legacy layout, remote `type` spellings), with
  an automatic backup and a `--dry-run` preview when a tool is carrying too many servers
- 🔀 **`diff`** — see exactly which servers two tools disagree about
- 🧬 **`sync`** — additive merge from one tool to another (never deletes;
  `--force` overwrites drifted entries)
- 🅿️ **`enable` / `disable`** — park a server without removing it, using the
  tool's own documented disable switch (Codex and Zed `enabled`, Cline and
  Roo Code `disabled`); parked servers are marked `(off)` in `list`, and
  `doctor` knows a parked server is intentional
- 📂 **`--project <dir>`** — operate on the configs checked into a repo:
  Claude Code `.mcp.json`, Cursor `.cursor/mcp.json`, VS Code
  `.vscode/mcp.json`, Gemini CLI `.gemini/settings.json`, Codex
  `.codex/config.toml`, opencode `opencode.json` — the same surgical edits,
  scoped to the project
- 🔐 **`audit`** — security check for your MCP configs: hardcoded credentials
  in env and header values (known token formats from the gitleaks/trufflehog
  rule sets: AWS, GitHub, OpenAI, Stripe, GCP, Slack, JWTs — plus a Shannon
  entropy heuristic for unprefixed ones, with placeholders and `${VAR}`
  references correctly skipped), and a Unix permission check that warns when
  a config full of credentials is readable by others
- 📡 **`--json`** — every read command (`scan`, `list`, `show`, `doctor`,
  `audit`) emits a single schema-versioned JSON document on stdout, with
  unchanged exit codes — pipe to `jq`, gate CI, build editor integrations
- 🪡 **Surgical edits** — `add`/`rm` write the *exact* dialect each tool
  expects, preserve every unrelated key, use atomic `write+rename`, and back up
  the previous file to `~/.mcpmedic/backups/` first
- 📦 **`export` / `import`** — portable JSON for dotfile repos, machine
  migration, and team onboarding
- 🛡️ **Read-only where writing is unsafe** — JSONC configs (with comments) and
  opencode are read and diagnosed but never rewritten
- ⚡ **Single static binary**, no Node runtime, no daemon, no config of its own
- ✅ **114 tests**, `clippy::pedantic` clean, cross-platform (Linux / macOS / Windows), 27 tools

## Supported tools

| Tool | Config (relative to home) | Dialect | Edit |
|---|---|---|---|
| Claude Code | `~/.claude.json` → `mcpServers` (user scope) | JSON | ✅ |
| Claude Desktop | platform app-support dir → `mcpServers` | JSON | ✅ |
| Cursor | `~/.cursor/mcp.json` → `mcpServers` | JSON | ✅ |
| Windsurf | `~/.codeium/windsurf/mcp_config.json` → `mcpServers` | JSON | ✅ |
| VS Code | `~/.vscode/mcp.json` → `servers` (with `type`) | JSON | ✅ |
| Zed | `~/.config/zed/settings.json` → `context_servers` | JSON | ✅ |
| Gemini CLI | `~/.gemini/settings.json` → `mcpServers` | JSON | ✅ |
| Codex CLI | `~/.codex/config.toml` → `[mcp_servers.*]` (honors `CODEX_HOME`) | TOML | ✅ |
| Cline | VS Code globalStorage `cline_mcp_settings.json` | JSON | ✅ |
| Roo Code | VS Code globalStorage `mcp_settings.json` | JSON | ✅ |
| opencode | `~/.config/opencode/opencode.json[c]` → `mcp` (v1 + v2 layouts) | JSONC | 👁 read-only |
| Warp | `~/.warp/.mcp.json` → `mcpServers` | JSON | ✅ |
| Kiro | `~/.kiro/settings/mcp.json` → `mcpServers` | JSON | ✅ |
| TRAE | platform app-support dir → `User/mcp.json` → `mcpServers` | JSON | ✅ |
| Antigravity | `~/.gemini/config/mcp_config.json` → `mcpServers` (`serverUrl`) | JSON | ✅ |
| LM Studio | `~/.lmstudio/mcp.json` → `mcpServers` | JSON | ✅ |
| Continue | `~/.continue/mcpServers/mcp.json` → `mcpServers` (JSON drop-in) | JSON | ✅ |
| GitHub Copilot CLI | `~/.copilot/mcp-config.json` → `mcpServers`; project `.github/mcp.json` | JSON | ✅ |
| Kimi Code | `~/.kimi-code/mcp.json` → `mcpServers`; project `.kimi-code/mcp.json` | JSON | ✅ |
| Qwen Code | `~/.qwen/settings.json` → `mcpServers` (`httpUrl` for HTTP remotes); project `.qwen/settings.json` | JSON | ✅ |
| Auggie | `~/.augment/settings.json` → `mcpServers` | JSON | ✅ |
| Factory Droid | `~/.factory/mcp.json` → `mcpServers`; project `.factory/mcp.json` | JSON | ✅ |
| Amp | `~/.config/amp/settings.json` → `amp.mcpServers` (flat dot-key); project `.amp/settings.json` | JSON | ✅ |
| Crush | `~/.config/crush/crush.json` → `mcp`; project `crush.json` | JSON | ✅ |
| OpenHands | `~/.openhands/mcp.json` → `mcpServers` | JSON | ✅ |
| Devin CLI | `~/.config/devin/config.json` → `mcpServers`; project `.devin/config.json` | JSON | ✅ |
| OpenClaw | `~/.openclaw/openclaw.json` → `mcp.servers` | JSON5 | 👁 read-only |

Claude Code's `CLAUDE_CONFIG_DIR` and Codex's `CODEX_HOME` environment
overrides are respected. Point `HOME`/`USERPROFILE` at another machine's home
directory to inspect it — handy for dotfile maintenance.

**Remote-server dialects are normalized across every tool**: Claude Code and
Claude Desktop key on `type: "http"`, Roo Code on the literal
`streamable-http`, Cline on `streamableHttp`, Gemini CLI on its `httpUrl`
field, Windsurf on `serverUrl`, and Cursor/Zed on a plain `url` with the
transport inferred. mcpmedic reads all of them, writes the right one when you
`add` or `sync`, and diffs the normalized form — so a server never counts as
drift just because two tools spell its transport differently.

## Install

**From crates.io (any platform with a Rust toolchain ≥ 1.85):**

```sh
cargo install mcpmedic
```

**From Homebrew (macOS / Linux — prebuilt binary, no Rust toolchain needed):**

```sh
brew install useless-rs/mcpmedic/mcpmedic
```

**From source:**

```sh
cargo install --git https://github.com/useless-rs/mcpmedic
```

**Or build locally:**

```sh
git clone https://github.com/useless-rs/mcpmedic
cd mcpmedic
cargo install --path .
```

Prebuilt binaries for Linux, macOS and Windows are attached to every
[release](https://github.com/useless-rs/mcpmedic/releases).

Homebrew users on macOS or Linux can install the prebuilt binary directly
(no Rust toolchain required):

```sh
brew install useless-rs/mcpmedic/mcpmedic
```

## Quickstart

```console
$ mcpmedic                       # scan: which tools exist, how many servers
mcpmedic — first aid for MCP configs

  ○ claude-code    ~/.claude.json
  ✓ cursor         ~/.cursor/mcp.json  2 servers
  ✓ opencode       ~/.config/opencode/opencode.jsonc  22 servers
  ○ vscode         ~/.vscode/mcp.json
  ...

$ mcpmedic list                   # every server, every tool, normalized
$ mcpmedic init                   # one-shot setup: detect + sync everywhere
$ mcpmedic doctor                 # what is broken, drifted or bloated
$ mcpmedic doctor --fix            # apply the safe repairs (backed up first)
$ mcpmedic show context7           # where is this server configured?
$ mcpmedic diff cursor windsurf   # the drift between two tools
```

Add a server to a tool — in *that tool's* dialect, with a backup:

```sh
# stdio server
mcpmedic add playwright --to cursor --command npx --env HEADLESS=1 -- -y @playwright/mcp

# remote HTTP server
mcpmedic add github --to zed --url https://api.githubcopilot.com/mcp/ --header "Authorization=Bearer $PAT"
```

```console
  ✓ `playwright` added (stdio) in Cursor
      ~/.cursor/mcp.json
      { "command": "npx", "args": [ "-y", "@playwright/mcp" ], "env": { "HEADLESS": "1" } }
      backup: ~/.mcpmedic/backups/cursor-1790190568842.json
```

Copy your Cursor setup into Windsurf, additively, without deleting anything:

```sh
mcpmedic sync --from cursor --to windsurf          # missing servers only
mcpmedic sync --from cursor --to windsurf --force  # also fix drifted ones
mcpmedic sync --from cursor --to windsurf --dry-run
```

Back up and move everything:

```sh
mcpmedic backup                                   # manual backup of all configs
mcpmedic export --out my-mcp-servers.json         # portable JSON
mcpmedic import my-mcp-servers.json              # restore into every tool
```

Install shell completions:

```sh
mcpmedic completions bash  > ~/.local/share/bash-completion/completions/mcpmedic  # bash
mcpmedic completions zsh   > ~/.zfunc/_mcpmedic                                    # zsh (+ compinit)
mcpmedic completions fish  > ~/.config/fish/completions/mcpmedic.fish             # fish
```

Work on a repo's checked-in configs (`.mcp.json` and friends):

```sh
mcpmedic scan --project .        # what MCP servers does this repo declare?
mcpmedic doctor --project .      # health-check the project setup
mcpmedic add github --to vscode --url https://api.githubcopilot.com/mcp/ --project .
```

## Commands

| Command | What it does |
|---|---|
| `scan` (default) | Detect installed tools and their configs, with server counts |
| `init` | One-shot onboarding: detect every configured tool, pick the richest as the source, sync it to every other installed tool. With `--json`: print the plan only, no files touched |
| `preset list` / `preset add <name> --to <t> [--force] [--dry-run]` | Curated zero-config bundles: `minimal` (memory + sequential-thinking) and `demo` (the official everything test server). Skips existing servers unless `--force` |
| `list [--tool <t>]` | Table of every server per tool |
| `show <name>` | Every tool that configures a given server + drift check |
| `doctor [--tool <t>] [--strict] [--fix] [--probe] [--explain]` | Health check: broken JSON, dead commands, dialect violations, cross-tool drift, bloat, unset `${VAR}` env references (e.g. `${GITHUB_TOKEN}` or `${env:API_KEY}` referenced in `env`/`headers` but not present in the environment), npx footguns (missing `-y` hang risk, `@latest` registry round-trips). `--fix` applies safe auto-repairs — including inserting the missing `npx -y` (`--dry-run` previews); `--probe` speaks MCP (probes run in parallel): stdio servers get a real initialize handshake plus a tools/list query — reporting the server's name, protocol version and exposed tool count; modern (2026-07-28-era) servers are identified via server/discover; a crashed server surfaces its stderr tail (the real failure cause); remotes get a TCP connect; `--explain` prints how-to-fix hints per category (`--json` findings carry a `fix` field) |
| `diff <a> <b>` | Server drift between two tools |
| `add <name> --to <t> ...` | Add a stdio (`--command ... -- args`) or remote (`--url`) server |
| `rm <name> --from <t>` | Remove one server |
| `enable` / `disable` <name> --from <t> | Park or resume a server without removing it, using the tool's own documented disable switch (`--dry-run` supported) |
| `sync --from <a> --to <b>` | Additive merge; `--force`, `--names`, `--dry-run` supported |
| `sync-all --from <a>` | Mirror one tool into every other detected tool (one-to-many); `--force`, `--names`, `--dry-run` supported |
| `export [--out <file>]` | Dump everything to portable JSON |
| `import <file> [--to <t>]` | Restore an export (per-tool sections or flat `servers` map) |
| `audit [--tool <t>]` | Security audit: hardcoded secrets in env/header values (known token formats + entropy heuristic), config file permissions |
| `completions <shell>` | Print completions for bash, zsh, fish, elvish or powershell |
| `backup [--tool <t>]` | Manual backup (also automatic before every mutation) |
| `restore [--tool <t>] [--list] [--latest]` | Restore configs from automatic backups; the current state is backed up first, so restores are reversible |
| `summary` | One-line health overview (tools, servers, findings) for shell prompts and CI gates; exits 1 on criticals |

All commands accept `--project <dir>` to operate on repo-checked-in configs
instead of user-global files; all mutation commands support `--dry-run`.
All read commands (`scan`, `list`, `show`, `doctor`, `audit`) accept `--json`
for schema-versioned machine output on stdout — exit codes are unchanged,
so `doctor --json` still exits 1 on critical findings and gates CI:

```sh
mcpmedic doctor --json | jq '.findings[] | select(.severity == "critical")'
mcpmedic scan --json   | jq '.tools[] | select(.state == "loaded") | .id'
```

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success / doctor found nothing critical |
| `1` | `doctor` found critical findings (or warnings with `--strict`) |
| `2` | Operational error (unknown tool, unparseable target config, …) |

`doctor` is CI-friendly: run it in a dotfiles pipeline and fail the build on
config rot.

## Safety model

1. **Atomic writes** — changes land in a temp file next to the target and are
   `rename`d into place; a crash can never leave a half-written config.
2. **Automatic backups** — the previous file is copied to
   `~/.mcpmedic/backups/<tool>-<epoch-millis>.<ext>` before every mutation.
3. **Surgical edits only** — mcpmedic never rewrites keys it doesn't own. Your
   `numStartups`, themes, model settings and comments (in TOML) are preserved.
4. **Read-only where risky** — configs with JSONC comments and opencode's
   moving-target format are parsed and diagnosed but never written.
5. **Refusal, not heroics** — a config with a parse error is reported, never
   edited.
6. **Additive sync** — `sync` never deletes servers from the target.
7. **Backup hygiene** — `~/.mcpmedic` is 0700 and every backup file 0600,
   because backups contain the same credentials as the configs they mirror.
   New config files `mcpmedic` creates get mode 600 on Unix; existing files
   keep their current mode.
8. **Security audit** — `mcpmedic audit` scans env and header values for
   hardcoded credentials and warns when a config is readable by group/others;
   it exits 1 when a secret is found, so it gates CI.
9. **Reversible restores** — `mcpmedic restore` backs up the current state
   before writing the backup's contents, so a restore can itself be undone
   with another `restore --latest`.

## Why not a GUI or a gateway?

Existing solutions are mostly macOS-only menu-bar apps, hosted gateways, or
npm scripts. mcpmedic is for everyone else: a 2 MB static binary that runs on
any OS, inside CI, over SSH, on a teammate's machine — no runtime, no daemon
between you and your tools, no config stored anywhere but the configs
themselves.

## Roadmap & backlog

Scored by impact (I 1-5), effort (E 1-5), risk (L/M/H). One item per
improvement cycle; log in [`CONTRIBUTING.md`](CONTRIBUTING.md#improvement-log).

| # | Item | I | E | Risk | Status |
|---|---|---|---|---|---|
| 1 | ~~Shell completions~~ (`mcpmedic completions bash\|zsh\|fish\|powershell`) | 4 | 2 | L | ✅ cycle 2 |
| 2 | ~~`doctor --fix` safe auto-repairs~~ (VS Code `type`, Zed legacy, remote `type` spellings) | 4 | 3 | M | ✅ cycle 6 |
| 3 | ~~Per-tool remote-transport dialects end-to-end~~ (read `httpUrl`/`serverUrl`, write each tool's exact remote shape) | 4 | 3 | M | ✅ cycle 7 |
| 4 | ~~`enable` / `disable` servers without removal~~ (Codex/Zed `enabled`, Cline/Roo `disabled`) | 3 | 3 | L | ✅ cycle 8 |
| 5 | ~~Project-scoped configs~~ (`--project <dir>`: `.mcp.json`, `.cursor/mcp.json`, `.vscode/mcp.json`, `.gemini/settings.json`, `.codex/config.toml`, `opencode.json`) | 4 | 4 | M | ✅ cycle 9 |
| 6 | ~~`--json` machine output for `scan` / `list` / `show` / `doctor` / `audit`~~ | 3 | 3 | L | ✅ cycle 12 |
| 7 | ~~More tools~~ Warp, Kiro, TRAE, Antigravity added (JetBrains excluded: their IDEs are MCP *servers*, not clients) | 3 | 2 | L | ✅ cycles 15-16 |
| 8 | Social preview PNG upload | 2 | 1 | L | ✅ cycle 10 (PNG ready in `docs/brand/`; upload in repo settings) |
| 9 | ~~Absolute image URLs before `cargo publish`~~ | 2 | 1 | L | ✅ cycle 13 |
| 10 | ~~MCP reachability probe in `doctor`~~ (stdio spawn 500ms, TCP connect 3s) | 4 | 4 | H | ✅ cycle 19 |

Explicit non-goal: telemetry, ever. A config doctor that phones home would be
a bad joke.

Contributions welcome — the [tool registry](src/registry.rs) is a single file
and adding a new tool is ~15 lines.

## Brand

mcpmedic's identity is **"a first-aid station in your terminal"**: the prompt
cross, a heartbeat line, monospace type, dark surfaces, and exactly one
accent color. Canonical assets live in [`docs/brand/`](docs/brand/) — use
them as-is; don't redraw or recolor them.

| Token | Hex | Role |
|---|---|---|
| Ink | `#0D1117` | Dark surfaces (logo/terminal background) |
| Paper | `#E6EDF3` | Wordmark and text on dark |
| Mint | `#2EE6A8` | The single brand accent: the cross, the heartbeat line, the chevron |
| Slate | `#8B949E` | Muted text, taglines |

**Isotype — "the prompt cross."** One solid, symmetric medic cross with the
terminal `>` chevron carved at its heart: the filled shape is the first-aid
kit, the negative space is the verb (open a prompt, get help). Construction:

- A single bold silhouette — no seams, no fragmentation. Reflection
  symmetry, equal arms: stability and trust, like the medical cross itself.
- The chevron tapers like the `>` glyph in a terminal; below ~24 px it fills
  in and the mark reads as a plain solid cross — the small-size tier is the
  same shape, not a redraw.
- Wordmark set in a monospace face, lowercase, always matching the command.

**Text-bearing assets ship as PNG** with the font baked in. SVG `<text>`
breaks on GitHub — the renderer substitutes fonts, shifting metrics and
baselines (the classic SVG-text trap); a baked PNG renders identically
everywhere. The SVGs in `docs/brand/` are sources, not shipping artifacts.

Rules distilled from the broader CLI brand canon (bat, eza, NixOS, and
friends):

- **Mint is the spark** — it marks brand moments only (logo, scan header,
  help literals). Success/warning/critical/info in `doctor` output are
  *semantic* colors (green/amber/red/blue) and are never recolored to mint.
- **Flat colors only** — no gradients, glows, shadows or 3D on the marks.
- No text inside the isotype — the carved chevron carries the terminal
  story as a shape, so it survives every size.
- Dark backgrounds are the default for brand surfaces.

Assets: [`banner.png`](docs/brand/banner.png) (README hero) ·
[`logo.png`](docs/brand/logo.png) (isotype) ·
[`favicon.png`](docs/brand/favicon.png) (tab tier) ·
[`social-preview.png`](docs/brand/social-preview.png) (1280×640 — upload in
repo *Settings → Social preview*). Sources: the matching `.svg` files.

## License

[MIT](LICENSE) © 2026 useless-rs
