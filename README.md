# mcp-tradebot

[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/CODERPP10/MCP-tradebot)

An MCP server that lets an LLM agent place orders on a personal Zerodha (Kite
Connect) account, with guardrails enforced structurally — independent of whatever
the LLM decides.

## Status

Pre-implementation. `DESIGN.md` is the agreed specification for a ground-up Rust
rewrite. No Rust code has been written yet.

The original TypeScript proof of concept lives on the **`archive/ts-prototype`**
branch, frozen as a behavioural reference only.

## Next step

`feat/workspace-skeleton` — cargo workspace, crate stubs, CI, `policy.toml` schema,
and the core domain types. See `DESIGN.md` §9 for the full branch plan.
