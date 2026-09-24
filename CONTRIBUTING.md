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
