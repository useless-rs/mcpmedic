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

