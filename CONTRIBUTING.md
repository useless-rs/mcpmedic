# Contributing to mcpmedic

Thanks for helping make MCP config first aid better. This project is improved
in small, continuous cycles — one improvement per cycle, always shipped green.

## Ground rules

- **Never break a working config.** The safety model (atomic writes, automatic
  backups, refusal to edit broken/JSONC files) outranks any feature.
- **Small, reversible changes.** One feature or fix per commit, tests included.
- **Stay a boring, scriptable binary.** No daemon, no telemetry (ever), no
  interactive-only flows that break piping.
- `clippy::pedantic` clean, `cargo fmt` applied, all tests green on all three
  OSes — CI enforces it, so run it locally first:
  ```sh
  cargo fmt && cargo clippy --all-targets --locked -- -D warnings && cargo test --locked
  ```
- Releases double as the changelog: every version gets notes on the
  [GitHub releases](https://github.com/useless-rs/mcpmedic/releases) page.

## Adding a tool to the registry

The whole tool landscape lives in [`src/registry.rs`](src/registry.rs):

1. Add a `ToolId` variant and a `ToolSpec` entry (display name, config format,
   path resolver). If the tool uses a known dialect (`mcpServers` JSON, VS Code
   `servers`, Zed `context_servers`, Codex TOML), you are done.
2. New dialect? Add a `Format` variant plus reader/writer functions in
   [`src/format.rs`](src/format.rs), including round-trip tests that prove
   unrelated keys and comments survive edits.
3. Add an end-to-end fixture test in [`tests/cli.rs`](tests/cli.rs) that adds a
   server, removes it, and confirms the file went back to byte-equivalent
   content (minus the changed entry).
4. Update the supported-tools table in the README.

Mark a tool `writable: false` (read + `doctor` only) when editing its file
would be unsafe — comments that a JSON rewrite would destroy, or a format that
is still moving.

## Adding a doctor check

Checks live in [`src/doctor.rs`](src/doctor.rs). Every finding needs:

- a severity that matches reality (`Critical` = the server almost certainly
  does not work; `Warning` = works today, surprises you soon; `Info` = worth
  knowing),
- a test with a fixture that triggers it,
- no false positives on healthy configs — a `doctor` that cries wolf is worse
  than no `doctor` at all.

## Improvement log

One line per improvement cycle: date, what changed, why.

- **2026-09-23 · cycle 1 — brand identity.** Added the isotype (medic cross +
  `>_` prompt), ECG heartbeat motif, mint-on-ink palette, branded clap help
  styles, and the README hero banner (`docs/brand/`). Why: every
  category-defining CLI (bat, eza, ripgrep) is instantly recognizable in a
  terminal screenshot; mcpmedic had no visual identity at all. Research:
  CLI/dev-tool brand guides (dark-first, monospace lowercase wordmark, one
  accent color used sparingly, flat marks that survive 16 px).
- **2026-09-23 · cycle 2 — shell completions.** `mcpmedic completions <shell>`
  prints scripts for bash, zsh, fish, elvish and powershell via `clap_complete`
  (AOT generation from the same derive definition that powers the CLI, so
  flags and help can never drift from completions). Why: completions are
  table stakes for CLI adoption — every peer tool ships them, and the
  `doctor`/`sync` verbs are long enough to earn them. Docs: README install
  snippets for the three common shells; e2e test asserts every script names
  the binary and rejects unknown shells.
- **2026-09-23 · cycle 3 — CI hygiene.** `actions/checkout` bumped v4 → v7
  (v4 targets the EOL Node.js 20 runtime; v5+ runs on node24 — our own CI
  logs carried the deprecation warning), and CI gained a `concurrency` group
  so superseded pushes cancel in-progress runs instead of burning runner
  minutes. Why: a green, warning-free CI page is part of the "trustworthy
  first-aid kit" pitch — warning noise erodes trust.
- **2026-09-23 · cycle 4 — v0.1.0 released.** Two release-blocker fixes found
  while exercising the never-run release pipeline: `action-gh-release` bumped
  v2 → v3 (Node 20 is removed from Actions runners on 2026-09-23 — the same
  day; v2 runs on Node 20), and the `aarch64-unknown-linux-gnu` leg gained
  the missing cross-linker it always needed. Tagged `v0.1.0`; the workflow
  shipped five ~0.8 MB binaries (linux amd64/arm64, macos amd64/arm64,
  windows), verified by downloading the linux-x86_64 archive and running
  `mcpmedic --version` and a live `list` against a real machine.
- **2026-09-23 · cycle 5 — isotype v2, "the cursor cross".** Redesigned the
  logo from craft principles (research: reduction/silhouette tests, one-idea
  marks with double readings, optical correction, grid rhythm). The v1 mark
  carved `>_` text into a plain cross — text inside a mark dies below 32 px
  and a bare cross is category-generic. The v2 mark is one idea: a medic
  cross built from five terminal cursor blocks with 10-unit seams, the
  horizontal arm optically thinned 11.5%, responsive by construction (the
  seams close below ~32 px and the mark heals into a solid cross). Added a
  dedicated favicon tier. Both behaviors verified by rendering at 512/64/16
  px and pixel-sampling: distinct blocks at 512, solid cross at 16.
- **2026-09-23 · cycle 6 — `doctor --fix`.** Closes the loop from diagnosis
  to cure, restricted to provably safe repairs: VS Code entries missing
  `type` (added by shape: `stdio`/`http`), Zed's legacy nested `command`
  object (flattened), and remote entries whose `type` spelling the target
  tool cannot read — Claude family needs `type: "http"`, Roo Code validates
  the literal `streamable-http`, Cline uses `streamableHttp`, and generators
  that emit a `transport` field produce configs those tools silently ignore
  (research: Claude Code's official docs + a per-client transport-field
  matrix). Hybrid command+url entries are never auto-fixed — they need human
  judgment. Read-only configs report skippable repairs instead of editing.
  Every repair is backed up before the atomic write; `--fix --dry-run`
  previews without touching files. The same dialect knowledge powers new
  doctor findings, so detection and repair can never disagree.
- **2026-09-23 · cycle 7 — per-tool remote-transport dialects, end to end.**
  The reader now accepts every remote URL field (`url`/`httpUrl`/`serverUrl`)
  — Gemini CLI and Windsurf configs were previously invisible to mcpmedic,
  surfacing as unparseable entries — and `add`/`sync` write each tool's exact
  documented remote shape: Claude tools `type: "http"`, Roo Code the literal
  `streamable-http`, Cline `streamableHttp`, Gemini `httpUrl`, Windsurf
  `serverUrl`, Cursor/Zed a plain `url` with the transport inferred. Export
  keeps one portable dialect (Claude's shape) for interchange. Why: official
  docs for Gemini CLI and Windsurf both warn that an entry copied from
  another client's example will not connect — dialect-correct writes are the
  difference between "added" and "silently broken". An invariant test now
  asserts doctor never flags a dialect mcpmedic itself wrote. The live
  validation demo also exposed a `diff` bug (identical URLs printed as
  "`url` vs `url`" when only headers differed); `diff` now names the
  differing headers, with a regression test.
- **2026-09-23 · cycle 8 — `enable` / `disable` without removal.** Parking a
  server now uses each tool's own documented disable switch, set only where
  research found one: Codex `enabled = false` (official config reference),
  Zed `"enabled": false` (the toggle's persisted shape, confirmed in Zed's
  tracker), Cline and Roo Code `"disabled": true` (official docs). Tools
  without a documented switch — Cursor, the Claude tools, Windsurf, Gemini
  CLI, and VS Code, whose enabled state lives outside `mcp.json` — refuse
  with a pointer to `rm`, keeping mcpmedic's strict evidence bar. `list` and
  `show` mark parked servers `(off)`, and `doctor` treats a parked server as
  intentional: a parked dead command is not a finding, a resumed one is
  flagged again. Every mutation is atomic with an automatic backup. Known
  gap: `sync`/`export` don't carry the flag (the normalized model is
  transport-only) — parked state stays tool-local until the model grows a
  state field. 8 new tests, 75 total.
- **2026-09-23 · cycle 9 — `--project` mode.** Every command accepts
  `--project <dir>` to operate on the project-scoped configs tools document
  for repos: Claude Code `.mcp.json`, Cursor `.cursor/mcp.json`, VS Code
  `.vscode/mcp.json`, Gemini CLI `.gemini/settings.json`, Codex
  `.codex/config.toml`, opencode `opencode.json` — first-party doc evidence
  for each. Windsurf, Cline and Roo Code stay out until their own docs
  confirm a project scope (third-party sync tools claim paths for them, but
  other sources say global-only; the strict evidence bar wins). In project
  mode, non-project tools act as not-installed and edits to them refuse;
  fresh project configs auto-create their parent directories (`.vscode/`,
  `.gemini/`); backups stay in the user-global `~/.mcpmedic/backups` so
  nothing pollutes the repo. The single path funnel (`ctx.path`/`ctx.load`)
  made this a contained change — `store::load` now takes the path
  explicitly. 2 new tests, 77 total.
- **2026-09-23 · cycle 10 — isotype v3 "the prompt cross", PNG assets.**
  The v2 "five cursor blocks" mark read as fragmented — the seams looked
  like rendering errors — and every banner used SVG `<text>`, which
  GitHub's renderer substitutes fonts under (the classic SVG-text trap).
  v3: one solid, symmetric cross — a single bold silhouette — with the
  terminal `>` chevron carved by absence at its heart (negative space as
  the verb; it fills to a plain cross below ~24 px). Text-bearing assets
  now ship as PNG with iA Writer Duospace Bold baked in; SVGs remain as
  sources. Verified by pixel-rendering the 16/32/512 px tiers.
- **2026-09-23 · cycle 11 — `mcpmedic audit` + backup hygiene.** A from-scratch
  secret scanner (zero new dependencies, per the independence mandate):
  known-token prefix signatures borrowed from the gitleaks/trufflehog rule
  sets (AWS `AKIA`, GitHub `ghp_`/`github_pat_`, OpenAI/Anthropic `sk-`,
  Stripe `sk_live_`, GCP `AIza`, Slack `xox`, JWT `eyJ`, private keys),
  a Shannon-entropy heuristic for unprefixed credential-shaped env values,
  and a placeholder filter that correctly skips `${VAR}` references and
  documentation stubs. `audit` also warns when a config file is
  group/world-readable on Unix. Backups are now hardened: `~/.mcpmedic`
  is 0700, backup files 0600, and newly created config files get mode 600
  (existing files keep their current mode — we don't silently change what
  the tool's owner set). 7 new unit tests + 1 e2e; 84 total.
- **2026-09-23 · cycle 12 — `--json` machine output.** Every read command
  (`scan`, `list`, `show`, `doctor`, `audit`) now emits a single
  schema-versioned JSON document on stdout when `--json` is passed. Exit
  codes are unchanged — `doctor --json` still exits 1 on critical findings
  and `audit --json` still exits 1 on hardcoded secrets — so the flag is a
  drop-in for CI gates and jq pipelines, not a behavioral change. Output
  carries `schema_version: 1` and a `generator` field per the CLI JSON
  contract research (data on stdout only, stable schema, deterministic
  ordering). One new e2e test validating all five commands produce
  parseable, schema-versioned JSON; 85 total.

- **2026-09-23 · cycle 13 — crates.io publish prep.** README hero banner
  switched to an absolute GitHub raw URL (crates.io doesn't resolve relative
  image paths), `homepage` added to Cargo.toml, install section now leads
  with `cargo install mcpmedic`. `cargo package` validates cleanly —
  139 KB .crate, all metadata fields verified (description, license,
  repository, readme, keywords all under 20 chars, valid category slugs,
  rust-version). The crate is ready to publish; the owner runs
  `cargo login <TOKEN>` + `cargo publish`.
- **2026-09-23 · cycle 14 — Homebrew tap.** Created
  [useless-rs/homebrew-mcpmedic](https://github.com/useless-rs/homebrew-mcpmedic)
  with a formula that downloads the prebuilt release binary — zero Rust
  toolchain needed by users. Covers macOS (arm64 + x86_64) and Linux
  (arm64 + x86_64) with sha256-verified tarballs from the v0.2.0 release.
  Install: `brew install useless-rs/mcpmedic/mcpmedic`.
- **2026-09-23 · cycle 15 — Warp, Kiro, TRAE added (14 tools).** Registry
  grows from 11 to 14 tools, each backed by first-party doc evidence:
  Warp (`~/.warp/.mcp.json`, project-scoped `{repo_root}/.warp/.mcp.json`),
  Kiro (`~/.kiro/settings/mcp.json`, project-scoped, documented `disabled`
  flag), TRAE (platform app-support dir → `User/mcp.json`). All three use
  the standard `mcpServers` JSON format, so every existing command works
  with them out of the box — scan, list, doctor, add, rm, sync, enable/
  disable, export, import, audit, `--json`, `--project`. JetBrains and
  Antigravity remain on the backlog. 1 new e2e test; 86 total.
- **2026-09-23 · cycle 16 — Antigravity added (15 tools).** Google's
  Antigravity IDE: `~/.gemini/config/mcp_config.json` with `serverUrl` for
  remote servers (like Windsurf), project-scoped `.agents/mcp_config.json`.
  Uses the standard `mcpServers` JSON format, so every existing command
  works immediately. JetBrains deliberately excluded after research: their
  IDEs are MCP *servers*, not clients — external tools connect TO them,
  and there is no documented JetBrains-side MCP client config file. The
  strict evidence bar wins. Backlog #7 is now complete. 86 tests.
- **2026-09-23 · cycle 17 — `mcpmedic restore`.** The undo button for
  mcpmedic's surgical edits. `restore` (or `restore --list`) shows
  available backups from ~/.mcpmedic/backups grouped by tool, newest
  first; `restore --latest [--tool <t>]` copies the most recent backup
  back to the config location. Restores are themselves reversible — the
  current config is backed up before the restore writes (via the same
  `persist()` used by every mutation), so `restore --latest` twice
  toggles between the two states. Research: the list → confirm →
  restore pattern from git stash / cfgd / stash-away. 1 new e2e test;
  87 total.
- **2026-09-23 · cycle 18 — v0.3.0 released.** Version bumped from 0.2.0,
  tagged, release workflow built all 5 platform binaries (linux amd64/arm64,
  macos amd64/arm64, windows). Homebrew formula updated with new sha256
  hashes. crates.io validated via cargo package (139 KB) but publish requires
  a token the owner must provide: `cargo login <TOKEN>` + `cargo publish`.
  Release notes enumerate all changes since v0.2.0.
- **2026-09-23 · cycle 19 — `doctor --probe`.** Reachability probe for
  every configured server, from scratch (zero new dependencies): stdio
  servers are spawned briefly (500 ms grace period, then killed if still
  running) and remote endpoints get a `TcpStream::connect_timeout` (3 s)
  with the host/port extracted from the URL via simple string parsing.
  Templated commands (`${VAR}`) are skipped. Findings: reachable → info,
  unreachable → critical (gates CI), skipped → info. Research: MCP health
  check patterns from mcp-health-monitor, mcp-healthcheck, mcp-pulse,
  mcp2cli doctor. 5 new unit tests + existing 35 e2e; 92 total.
- **2026-09-23 · cycle 20 — LM Studio added (16 tools).** LM Studio
  (~/.lmstudio/mcp.json) follows Cursor's `mcpServers` JSON notation per
  the official docs at lmstudio.ai/docs/app/mcp. Standard format — every
  existing command works immediately. 92 tests.
- **2026-09-23 · cycle 21 — `mcpmedic sync-all`.** One-to-many sync:
  mirror one tool's servers into every other detected tool in a single
  command. `sync-all --from cursor` copies cursor's servers into Claude
  Code, VS Code, Windsurf, Gemini CLI, etc. — each in the target's own
  dialect. `--names` filters, `--force` overwrites drift, `--dry-run`
  previews. Additive by design — never deletes. 92 tests.
- **2026-09-23 · cycle 22 — `mcpmedic summary`.** One-line health
  overview for shell prompts, CI gates, and quick checks:
  `mcpmedic: 16 tools · 5 configured · 23 servers · 2 parked · 0 critical
  · 2 warnings`. Exits 1 on critical findings (gates CI). `--json`
  emits schema-versioned output with a `healthy` boolean. `--project`
  scopes to repo configs. Research: git status --short pattern,
  kubectl --no-headers, "return only what matters" from agent-tools
  engineering. 92 tests.
- **2026-09-24 · cycle 23 — v0.4.0 released.** Version bumped from 0.3.0,
  tag pushed, release workflow built all 5 platform binaries. Homebrew
  formula updated with correct sha256 hashes. Release notes enumerate all
  changes since v0.3.0: doctor --probe, sync-all, summary, LM Studio.
  crates.io validated via cargo package (139 KB) but publish requires a
  token the owner must provide: `cargo login <TOKEN>` + `cargo publish`.
- **2026-09-24 · cycle 24 — Continue added (17 tools).** Continue dev
  IDE: ~/.continue/mcpServers/mcp.json — Continue's JSON drop-in
  compatibility mode (it auto-picks up standard `mcpServers` JSON files,
  the same format as Claude Desktop, Cursor, and Cline). Project-scoped
  at .continue/mcpServers/mcp.json. Continue's primary config is YAML
  (config.yaml with mcpServers as a list, not a map) — the JSON drop-in
  is the documented compatibility path for users coming from other tools.
  92 tests.
- **2026-09-24 · cycle 25 — doctor env var validation.** doctor now
  warns when a config references ${VAR} or ${env:VAR} in a server's env
  map or headers but that variable is not set in the current environment
  — the classic "configured but fails at runtime: missing API key"
  failure mode, caught before the IDE ever spawns the server. Template
  extraction handles ${VAR} (Warp/Claude style), ${env:VAR} (VS Code
  style), multiple references per value, and rejects invalid POSIX
  names (leading digit, symbols). Warning severity only: the var may be
  set when the IDE actually runs the server. 93 tests.

- **2026-09-24 · cycle 26 — `mcpmedic init` — one-shot onboarding.**
  Zero prompts (per clig.dev: "never require a prompt"): init detects
  every configured tool, picks the richest config as the source (most
  servers; ties keep registry order), and syncs it to every other
  installed tool via the sync-all machinery — only tools whose config
  directories exist are written, so nothing is created for tools the
  user has not installed. No configs found → prints a working
  `mcpmedic add` example and tells the user to re-run init afterwards.
  --json prints the detection report only (candidates, source, next
  command) without touching any file. 96 tests.
- **2026-09-24 · cycle 27 — v0.5.0 released.** Version bump 0.4.0 →
  0.5.0 shipping three features: Continue support (17 tools, JSON
  drop-in at ~/.continue/mcpServers/mcp.json), doctor env var
  validation (${VAR} / ${env:VAR} references checked against the
  environment), and `mcpmedic init` (zero-prompt onboarding: detect,
  pick the richest source, sync to every installed tool). 5-platform
  binaries (linux amd64/arm64, macOS amd64/arm64, Windows x86_64),
  Homebrew tap updated with v0.5.0 sha256 hashes. 96 tests.
- **2026-09-24 · cycle 28 — doctor --probe speaks MCP.** The stdio
  probe now performs a real MCP JSON-RPC initialize handshake: spawn,
  write a spec-conformant NDJSON initialize request (protocolVersion
  2025-11-25, clientInfo mcpmedic + CARGO_PKG_VERSION), scan stdout
  for the first JSON-RPC message (banner lines skipped), and verify
  the reply carries result.serverInfo + protocolVersion. A successful
  handshake reports the server's own identity: "probe: MCP handshake
  ok — `mock` speaks protocol 2025-11-25". Silent-but-running servers
  stay Info, not false criticals (npx startup can take seconds; 3s
  handshake timeout). JSON-RPC error replies and non-NDJSON stdout
  fall back to reachable-with-reason. Remotes keep TCP connect. Zero
  new dependencies. 100 tests (61 unit + 39 e2e).
- **2026-09-24 · cycle 28b — Windows probe race fix.** The EOF branch
  of the handshake probe now retries try_wait for up to 500 ms (10 ms
  polls): on Windows the child's stdout pipe can close a moment before
  the exit is observable via try_wait, which misclassified an
  instantly-crashing server (`sh -c 'exit 1'`) as Reachable. Caught by
  the windows-latest CI job; Linux/macOS observe the exit immediately.
- **2026-09-24 · cycle 29 — npx footgun detection.** doctor now warns
  on the two npx patterns behind the most common real-world MCP
  failure (-32001: Request timed out, per fixmcp's origin story and
  community reports): (1) `npx` without `-y`/`--yes` — hangs waiting
  for interactive confirmation when the package is not cached, and
  (2) any stdio arg containing `@latest` — forces an npm registry
  round-trip on every launch; pin an exact version instead. Both are
  static warnings (Warning severity), same pattern as the C25 env
  var validation. 104 tests (64 unit + 40 e2e).
- **2026-09-24 · cycle 30 — v0.6.0 released.** Version bump 0.5.0 →
  0.6.0 shipping two features: doctor --probe now speaks MCP (real
  JSON-RPC initialize handshake over stdio that reports each server's
  own name and protocol version, plus the Windows EOF-before-exit
  race fix) and npx footgun detection (npx without -y hang risk,
  @latest registry round-trips — the #1 cause of -32001 timeouts).
  5-platform binaries (linux amd64/arm64, macOS amd64/arm64, Windows
  x86_64), Homebrew tap updated with v0.6.0 sha256 hashes. 104 tests
  (64 unit + 40 e2e).
- **2026-09-24 · cycle 31 — tools/list probe query.** After a
  successful initialize handshake, the stdio probe now completes the
  MCP lifecycle: sends notifications/initialized, requests tools/list,
  and reports the count — "probe: MCP handshake ok — `mock` speaks
  protocol 2025-11-25, exposes 2 tool(s)". The stdout reader became a
  message pump streaming all JSON-RPC responses (banners/notifications
  still skipped) — required for multi-request conversations. The
  tools/list response is matched by id via a small ToolsReply enum
  (NotIt / Count / Unknown); unrelated messages are ignored until the
  2s deadline. Best-effort: a server that exits or stays silent after
  initialize is still McpOk, just without the count. 105 tests
  (65 unit + 40 e2e).
- **2026-09-24 · cycle 32 — stderr tail capture.** When a stdio server
  exits during the probe, its stderr output is now included in the
  finding — "unreachable: process exited with status 3 before
  answering MCP initialize — stderr: fatal: missing module" — so users
  see the real failure cause (missing dependency, missing credentials,
  bad path) instead of a bare exit code. A collector thread drains the
  child's stderr into a capped tail (300 chars, lines joined with
  " | "); the exit-confirmed paths receive it via a channel handoff
  (150ms budget, best-effort — orphaned grandchildren holding the
  pipe simply yield no tail). Live-but-silent servers are unaffected.
  Per the anthropics/claude-code#64541 "silent no-spawn" report, this
  is the difference between guessing and knowing. 106 tests
  (66 unit + 40 e2e).
- **2026-09-24 · cycle 33 — v0.7.0 released.** Version bump 0.6.0 →
  0.7.0 shipping two probe features: the tools/list query (after a
  successful initialize handshake the probe sends notifications/
  initialized + tools/list and reports each server's exposed tool
  count) and stderr tail capture (a crashed server's stderr output is
  included in the finding, so users see the real failure cause
  instead of a bare exit code). doctor --probe is now the deepest
  health check in the ecosystem: initialize handshake, tools/list
  with counts, stderr tails, env var validation, npx footguns, dead
  commands, dialect violations, drift, bloat. 5-platform binaries
  (linux amd64/arm64, macOS amd64/arm64, Windows x86_64), Homebrew
  tap updated with v0.7.0 sha256 hashes. 106 tests (66 unit + 40 e2e).
- **2026-09-24 · cycle 34 — preset server bundles.** `mcpmedic preset
  list` shows curated zero-config bundles; `mcpmedic preset add
  <name> --to <tool>` installs one (reusing the add machinery: resolve,
  load_mutable, write_entry, commit with automatic backup). Two bundles
  built from the still-maintained official reference servers:
  `minimal` (memory + sequential-thinking) and `demo` (the everything
  test server — pairs perfectly with doctor --probe's handshake and
  tools/list query). Existing servers are skipped unless --force;
  --dry-run previews; a next-step hint points at doctor --probe and
  sync-all. 111 tests (68 unit + 43 e2e).
- **2026-09-24 · cycle 35 — doctor --fix repairs npx.** The --fix
  flag now inserts the missing `-y` at the front of any npx server's
  args — auto-confirming the install prompt instead of hanging (the
  #1 real-world MCP failure per the C29 research; --yes first-in-args
  is the canonical position per real-world 2026 configs). The repair
  lives in a format-agnostic helper (fix_npx_yes) called from
  fix_json_config, so the existing --dry-run preview, read-only-config
  skip and automatic backup plumbing all apply unchanged. Entries that
  already pass -y/--yes, or npx with no args (nothing to
  auto-confirm), are untouched. 113 tests (69 unit + 44 e2e).
- **2026-09-24 · cycle 36 — v0.8.0 released.** Version bump 0.7.0 →
  0.8.0 shipping two features: preset server bundles (`mcpmedic
  preset list` / `preset add <name> --to <tool>` — curated zero-config
  bundles built from the still-maintained official reference servers:
  minimal = memory + sequential-thinking, demo = the everything test
  server) and the npx auto-repair (doctor --fix inserts the missing
  -y at the front of npx args — auto-confirming the install prompt
  instead of hanging, with --dry-run preview and automatic backup).
  5-platform binaries, Homebrew tap updated with v0.8.0 sha256 hashes.
  113 tests (69 unit + 44 e2e). Roadmap note: research identified
  GitHub Copilot CLI (~/.copilot/mcp-config.json, standard mcpServers)
  as the next registry candidate, plus Amp, Crush, Factory Droid,
  Kimi Code and Qwen Code.
- **2026-09-24 · cycle 37 — GitHub Copilot CLI added (18 tools).**
  GitHub's official agentic CLI: ~/.copilot/mcp-config.json with the
  standard mcpServers format (per the official GitHub docs — the same
  config `copilot mcp add` writes). Project-scoped at
  .github/mcp.json, the documented repo-shared location (Copilot
  also reads generic .mcp.json walking up to the repo root, which
  mcpmedic already covers via Claude Code's project scope). No
  documented disable flag → none. README stats line also refreshed
  (92 → 113 tests, stale since cycle 23). 113 tests (69 unit +
  44 e2e).
- **2026-09-24 · cycle 38 — Kimi Code, Qwen Code and Auggie added
  (21 tools).** Three new agents, all verified against first-party
  docs: Kimi Code (~/.kimi-code/mcp.json, standard mcpServers, an
  `enabled: false` disable flag like Codex/Zed, project
  .kimi-code/mcp.json, plain-url remotes like Cursor); Qwen Code
  (~/.qwen/settings.json, Gemini-style `httpUrl` dialect for HTTP
  remotes, project .qwen/settings.json); and Auggie — Augment Code's
  CLI (~/.augment/settings.json, standard mcpServers with
  type:http+url remotes, matching the existing catch-all write arm).
  The remote-write dialect match gained Qwen Code (httpUrl) and
  Kimi Code (plain url); the registry tests gained entries, and the
  disable negative list now covers every flagless tool including
  Continue, LM Studio and Copilot. 113 tests (69 unit + 44 e2e).
- **2026-09-24 · cycle 39 — v0.9.0 released.** Version bump 0.8.0 →
  0.9.0 shipping the 18→21 tool expansion: GitHub Copilot CLI
  (~/.copilot/mcp-config.json, project .github/mcp.json), Kimi Code
  (~/.kimi-code/mcp.json, enabled:false disable flag, project
  .kimi-code/mcp.json), Qwen Code (~/.qwen/settings.json, httpUrl
  remote dialect, project .qwen/settings.json) and Auggie
  (~/.augment/settings.json) — every one verified against
  first-party docs, with per-tool remote-write dialects wired
  (Qwen httpUrl, Kimi plain url). 5-platform binaries, Homebrew
  tap updated with v0.9.0 sha256 hashes. 113 tests (69 unit +
  44 e2e). Roadmap: Factory Droid (standard mcpServers + disabled
  flag, simple), Amp (flat "amp.mcpServers" dot-key — needs a new
  Format variant) and Crush ("mcp" key) researched for cycle 40.
- **2026-09-24 · cycle 40 — Factory Droid added (22 tools).**
  Factory's Droid CLI: ~/.factory/mcp.json, standard mcpServers
  (per the first-party docs.factory.ai research from cycle 39),
  project-scoped at .factory/mcp.json (the committed-to-repo
  location). Documented `disabled: boolean` disable flag — same
  shape as Cline/Roo/Kiro. Remotes use type:http + url, already
  the catch-all write arm; stdio `type` is optional in Droid, so
  no dialect wiring needed. 113 tests (69 unit + 44 e2e).
- **2026-09-24 · cycle 41 — Amp and Crush added (24 tools).** Two new
  agents with new config formats: Amp (~/.config/amp/settings.json
  with a flat "amp.mcpServers" dot-key — the dot is part of the key
  name, flowing through the generic servers_key path; project
  .amp/settings.json; plain-url remotes match the default write arm)
  and Crush — Charm's agent (~/.config/crush/crush.json with an
  "mcp" key and explicit type fields; project crush.json; documented
  `disabled` disable flag like Cline/Roo). The Format enum gained
  Amp and Crush variants; build_json_entry writes type:stdio for
  Crush stdio entries (like VS Code) and Crush remotes join the
  type:http+url catch-all; the npx -y repair covers both via the
  format-agnostic path; json_snippet learned both keys. First unit
  test for the new formats (parses_amp_and_crush_formats).
  114 tests (70 unit + 44 e2e).
- **2026-09-24 · cycle 42 — v0.10.0 released.** Version bump 0.9.0 →
  0.10.0 shipping the 21→24 tool expansion: Factory Droid
  (~/.factory/mcp.json, standard mcpServers, disabled flag, project
  .factory/mcp.json), Amp (~/.config/amp/settings.json — first tool
  with a flat "amp.mcpServers" dot-key, new Format variant, project
  .amp/settings.json) and Crush (~/.config/crush/crush.json — "mcp"
  key with explicit type fields, new Format variant, disabled flag,
  project crush.json). Two new config formats entered the Format
  enum, build_json_entry and json_snippet. 5-platform binaries,
  Homebrew tap updated with v0.10.0 sha256 hashes. 114 tests
  (70 unit + 44 e2e). Roadmap: OpenHands (~/.openhands/mcp.json,
  standard mcpServers — confirmed) and Devin CLI
  (~/.config/devin/config.json + project .devin/config.json —
  confirmed) researched for cycle 43.
- **2026-09-24 · cycle 43 — OpenHands and Devin CLI added (26
  tools).** Both verified against first-party docs (the cycle 42
  research): OpenHands (~/.openhands/mcp.json, standard mcpServers —
  the same file the OpenHands UI writes; user-level only, no
  documented project scope or disable flag) and Devin CLI
  (~/.config/devin/config.json, standard mcpServers; project-scoped
  at .devin/config.json, the committed-to-repo location — the
  gitignored .devin/config.local.json override is deliberately not
  mcpmedic's business). Plain registry additions — zero format
  changes needed. 114 tests (70 unit + 44 e2e).
- **2026-09-24 · cycle 44 — doctor --explain.** A new --explain flag
  appends a "How to fix" section after the findings, keyed by the
  problem categories present in the run (the smcp-doctor pattern:
  every finding class with a repair gets the exact repair path;
  healthy-probe and skip findings carry none). The hints are
  action-oriented — dead commands point at `mcpmedic rm`, missing
  npx -y at `doctor --fix`, env vars at the shell profile, drift at
  `mcpmedic diff`/`sync`, bloat at `mcpmedic disable`, probe crashes
  at the stderr tail + `--probe --tool`, dialect violations at
  `doctor --fix`. JSON findings now carry a per-finding `fix` field
  unconditionally (the fixmcp machine-actionable pattern). Hints
  are deduplicated per run. 116 tests (71 unit + 45 e2e).
