# Project status

_Last updated: 2026-09-06_

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

1. **`feat/workspace-skeleton`** — _done_ (PR #1). Cargo workspace, crate stubs,
   CI (fmt / clippy / test), `policy.toml` schema + `tradebot policy check`,
   domain types (`Money`, `Quantity`, `Side`, `OrderIntent`, `Decision`, `Reason`).
2. **`feat/kite-client`** — _in progress._ Async REST client (`reqwest`/`rustls`):
   auth header, `SHA256` session checksum, hand-rolled token-bucket rate limiter,
   typed models, `KiteError` taxonomy, no logging / no retries. Methods:
   `generate_session`, `place_order`, `cancel_order`, `orders`, `order_history`,
   `positions`, `holdings`, `margins_equity`, `quote`, `ltp`, `profile`,
   `instruments` (CSV). Tested against `wiremock` fixtures (no live calls).
3. **`feat/secret-store`** — _in progress (demo-first slice, branched off
   `feat/kite-client`)._ `SealedFileStore`: Argon2id-derived key,
   per-record XChaCha20-Poly1305 with the record name as AEAD associated data,
   verifier record for fast wrong-passphrase detection, atomic file replace,
   0600 on unix, key zeroized on drop. `tradebot init` wired (hidden prompts on
   a tty, piped stdin otherwise). `TRADEBOT_HOME` resolution.
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

- Rust toolchain installed (rustup, stable `1.98.1`, pinned in
  `rust-toolchain.toml`). Skeleton verified: `cargo build` / `clippy -D warnings`
  / `test` (11 pass) / `fmt --check` all green; `tradebot policy check` works.
- Open the PR for `feat/workspace-skeleton` → `main` and set `main` as the
  GitHub default branch + branch protection (gh CLI not installed).
- BFG history scrub (owner: user, later).
