# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

An MCP server exposing personal Zerodha (Kite Connect) trading to an LLM agent,
with structurally-enforced guardrails. **`DESIGN.md` is the authoritative spec** —
read it before implementing anything. This is a ground-up Rust rewrite; the
TypeScript prototype was reference-only and is frozen on `archive/ts-prototype`
(do not port it, do not build on it).

## Current state

Pre-implementation. No cargo workspace exists yet. The immediate task is branch 1
of `DESIGN.md` §9: `feat/workspace-skeleton`.

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
