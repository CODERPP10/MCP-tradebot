# Project status

_Last updated: 2026-09-02_

## Where things stand

The project is mid-transition from a throwaway TypeScript prototype to a
security-focused **Rust rewrite**. `DESIGN.md` is the authoritative spec for the
rewrite; this document tracks execution state.

### Repository layout (after the 2026-09-02 reorg)

| Ref | Purpose |
|---|---|
| `main` | Active development for the Rust rewrite. Protected, always builds. |
| `archive/ts-prototype` | Frozen TypeScript prototype — MCP server + Kite wrapper + a basic React/Express UI. **Behavioural reference only; do not port or extend.** |
| tag `v0.1.0-prototype` | Marks the final prototype commit (`11f5380`). |
| `Dev` | Superseded (equals the archive tip). Left in place, not for development. |
| `Basic-Trade` | Old stale prototype branch. |

### Credentials

- **API secret**: rotated 2026-09-02. The new secret must never be committed —
  it belongs in the sealed `SecretStore` (DESIGN.md §2.2), or a local gitignored
  `.env` for prototype runs.
- **API key**: unchanged. It is a public identifier and is inert without the
  secret, so rotation was deemed unnecessary.
- **History**: the old hardcoded key + secret remain in these commits, reachable
  from `main`, `Dev`, `Basic-Trade`, and the archived history:
  `0f9c1b8`, `44ba123`, `8d5f663`, `64100fd`, `936c820`, `a38aac9`, `ac885dd`,
  `2bfbd10`. **Action pending:** scrub with BFG before the repo is shared or made
  public. Non-urgent (secret is rotated) but not harmless (key still leaks).

## Roadmap

Follows `DESIGN.md` §9. Branch order:

1. **`feat/workspace-skeleton`** — _in progress._ Cargo workspace, crate stubs,
   CI (fmt / clippy / test), `policy.toml` schema + `tradebot policy check`,
   domain types (`Money`, `Quantity`, `Side`, `OrderIntent`, `Decision`, `Reason`).
2. `feat/kite-client` — REST client + typed models + fixture-based tests.
3. `feat/secret-store` — `SealedFileStore`, `tradebot init`.
4. `feat/session-auth` — `TokenProvider` / `ManualPaste`, `tradebot login`.
5. `feat/ledger` — SQLite schema, migrations, append/query API.
6. `feat/guardrails` — policy engine + exhaustive unit tests (heaviest coverage).
7. `feat/engine` — orchestration, reconciliation loop, `PaperExecutor`.
8. `feat/telegram-confirm` — bot long-poll, inline buttons, nonce/TTL, fail-closed.
9. `feat/mcp-server` — `rmcp` stdio server, tool schemas.
10. `feat/operator-cli` — `status` / `kill` / `resume` / `ledger` / `reconcile` /
    `confirm` / `reject`.
11. `integration/mvp` — end-to-end paper-mode test, then → `main`.

3–6 can proceed in parallel after 1. 7 depends on 2/4/5/6. 8–10 depend on 7.

## Immediate blockers / setup needed

- **Rust toolchain not installed** on the dev machine. Install via
  [rustup](https://rustup.rs/); the workspace pins its version in
  `rust-toolchain.toml`. Until then, skeleton code is authored but unverified
  (`cargo build` / `clippy` / `test` have not been run).
