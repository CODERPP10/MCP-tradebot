# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

An MCP server exposing personal Zerodha (Kite Connect) trading to an LLM agent,
with structurally-enforced guardrails. **`DESIGN.md` is the authoritative spec** —
read it before implementing anything. This is a ground-up Rust rewrite; the
TypeScript prototype was reference-only and is frozen on `archive/ts-prototype`
(do not port it, do not build on it).

## Current state

`feat/workspace-skeleton` (DESIGN.md §9 branch 1) in progress: cargo workspace,
crate stubs, CI, `policy.toml` + `tradebot policy check`, domain types. Most crates
are stubs that return a `NotImplemented` error / fail closed.

## Build & commands

Rust workspace (edition 2021, toolchain pinned in `rust-toolchain.toml`). Install
Rust via [rustup](https://rustup.rs/).

- `cargo build --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo fmt --all`
- `cargo run -p operator-cli -- policy check` — validate `policy.toml`, print effective values
- `cargo run -p operator-cli -- --help` — see all `tradebot` subcommands (most stubbed)

## Workspace crates (`crates/`)

`domain` (leaf: money/quantity/side/intent/decision) ← `guardrails` (policy engine,
must NOT depend on kite-client/engine/mcp-server) ← `engine` (sole caller of
kite-client mutating methods) ← `mcp-server` (bin `tradebot-mcp`) / `operator-cli`
(bin `tradebot`). Also `kite-client`, `secret-store`, `session`, `ledger`,
`telegram-confirm`.

## Branching

- `main` — protected, always builds; development target for the rewrite.
- `archive/ts-prototype` — frozen TS proof of concept, reference only.
- Feature branches follow `DESIGN.md` §9 (`feat/kite-client`, `feat/guardrails`, …).

## Non-negotiables from DESIGN.md

- `guardrails` is a separate crate that does **not** depend on `mcp-server`. There
  must be no compile path from a tool to a Kite order endpoint that skips
  `engine::submit()` → `guardrails::evaluate()`.
- No "raw passthrough" tool. The LLM only ever produces a schema-validated `OrderIntent`.
- Secrets live in the passphrase-sealed `SecretStore`, never in code, env, or logs.
- `Policy` comes from an operator-owned file the server process cannot write.
- Paper-mode by default; `--live` is a deliberate opt-in.
- Never auto-retry a mutating Kite call.

## Credentials

The API secret was rotated after the prototype exposed it. Old key/secret/tokens
remain in git history (to be cleaned with BFG later) but are inert. Never commit
the new secret.
