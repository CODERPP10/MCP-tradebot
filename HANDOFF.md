# Handoff — moving this repo to a managed sandbox

Written for whoever (human or agent) picks this up on a fresh machine. For
*what the system is*, read `DESIGN.md`; for *how to work in the repo*, read
`CLAUDE.md`; for *where development got to*, read `STATUS.md`.

Authority order, highest first: **`DESIGN.md` → `CLAUDE.md` → `STATUS.md` →
this file.** If this file ever disagrees with `DESIGN.md`, `DESIGN.md` wins
and this file is stale.

## 1. What state the code is in

Branch `feat/engine-reads` carries the whole safety chain. As of the last
commit the workspace is green:

```
cargo fmt --all -- --check                              # clean
cargo clippy --workspace --all-targets -- -D warnings   # 0 warnings
cargo test --workspace                                  # 150 passed
```

Roadmap items 1–12 of `DESIGN.md` §9 are implemented except: no live
executor, no `modify_order`, and `integration/mvp` has its end-to-end test but
has not been merged to `main`. Paper mode is the only mode that exists —
`tradebot-mcp serve --live` is refused in code, not just by configuration.

Run these to see it working, in this order:

```
cargo run -p operator-cli -- policy check        # no secrets needed
cargo run -p operator-cli -- --help              # every subcommand is real
cargo test --workspace
```

## 2. What does NOT travel with the repo

**No credential is in git and none may ever be added.** The sealed store, the
ledger and the instrument cache all live in `TRADEBOT_HOME` (default
`./.tradebot`), which `.gitignore` excludes along with `secrets.json`,
`*.sealed`, `tradebot.db*` and the holidays file.

A fresh sandbox therefore starts with **no account access** and must be
re-provisioned by the operator, interactively:

```
cargo run -p operator-cli -- init                      # prompts: api key, api secret, passphrase
cargo run -p operator-cli -- secrets put telegram_bot_token
cargo run -p operator-cli -- secrets put telegram_chat_id
cargo run -p operator-cli -- login                     # prints Kite login URL, takes pasted request_token
cargo run -p operator-cli -- whoami                    # proves the session works (two read-only calls)
```

Do **not** copy `.tradebot/` between machines as a shortcut unless the
passphrase moves by a channel that is not this repo and not a chat log. The
Kite API secret in there was already rotated once because the TypeScript
prototype leaked it; old keys remain in git history (inert, BFG scrub still
pending) and must not be reused.

`init` and `login` need a TTY (hidden passphrase input, pasted token), so an
unattended agent cannot complete them. Expect to do this step yourself.

### Known Windows gotcha
`rpassword` can leave the console in a bad mode (QuickEdit) such that the next
prompt appears to hang. Closing and reopening the terminal fixes it. If the
sandbox is Linux this stops being a problem.

## 3. Environment the sandbox needs

| Requirement | Value / note |
|---|---|
| Rust toolchain | `1.98.1`, pinned in `rust-toolchain.toml` (install via rustup; `rustfmt` + `clippy` components) |
| C toolchain | needed by `rusqlite`'s bundled SQLite |
| Network egress | `api.kite.trade`, `kite.zerodha.com`, `api.telegram.org` |
| `TRADEBOT_HOME` | data dir; defaults to `./.tradebot` |
| `TRADEBOT_PASSPHRASE` | **only** for `tradebot-mcp serve` — Claude Desktop cannot answer a prompt. A paper-demo shortcut (`DESIGN.md` §10); live mode must move this to an OS credential store. |
| `TRADEBOT_POLICY_PATH` | optional override for `policy.toml` |
| `policy.toml` | operator-owned, must not be writable by the server process |

The repo root still contains `frontend/` and `node_modules/` from the frozen
TypeScript prototype. Nothing in the Rust workspace reads them; they can be
left alone or dropped. The prototype itself is reference-only on
`archive/ts-prototype` — do not port it.

## 4. Rules an autonomous agent must not break

These are `DESIGN.md` non-negotiables, restated because a managed agent will
hit all of them:

1. No compile path from an MCP tool to a Kite order endpoint that skips
   `engine::submit()` → `guardrails::evaluate()`. `guardrails` must not depend
   on `mcp-server`, `engine` or `kite-client`.
2. No raw-passthrough tool. The LLM only ever emits a schema-validated
   `OrderIntent`.
3. Never auto-retry a mutating Kite call. Reconciliation marks a silent submit
   failure `submit_failed` and stops; a human decides what happens next.
4. Secrets only ever live in the sealed `SecretStore` — never in code, env
   files, logs, commits, or an agent's transcript.
5. Paper mode by default. Enabling `--live` is a deliberate human change that
   also requires a live executor (does not exist) and real `policy.toml`
   numbers (not written yet).
6. `policy.toml` is operator-owned. An agent may read it and report on it; it
   must not raise a cap to make a test or a trade pass.
7. `main` is protected. Work on feature branches per `DESIGN.md` §9.

## 5. Work queue, in order

1. **First real Telegram round-trip.** `cargo run -p mcp-server --example
   smoke_confirm -- <HELD_SYMBOL>` and tap the button. Needs a held symbol
   because the Kite app lacks the market-data add-on (see §6). Human-only.
2. **Open the PR chain to `main`** (`feat/engine-reads` → `main`), set `main`
   as the GitHub default branch, enable branch protection. Needs `gh` or the
   web UI; `gh` was not installed on the old machine.
3. **Real `policy.toml` numbers.** Currently the `DESIGN.md` placeholder.
   Operator decision, not an agent's.
4. **Live executor + `--live`**, only after 1–3 and a deliberate review.
   `DESIGN.md` §8 risk 7 (SEBI algo-trading obligations) applies before any
   real order.
5. `modify_order`, and a `policy.toml` content hash that survives
   reformatting (today's hash is over file bytes).

## 6. Standing environment limitation

The Kite Connect app has **no market-data add-on**, so `/quote` returns
`PermissionException`. `engine::submit` falls back to the symbol's
`last_price` from `positions`, then `holdings`. For an unheld symbol there is
no price, `ltp` stays `None`, and guardrails rejects `QUOTE_STALE` — the
price-band check is never silently skipped. Practical effect: **only held
symbols are tradeable** until the add-on is bought.

## 7. Verifying a migration worked

1. `cargo test --workspace` → 150 passed, and
   `cargo clippy --workspace --all-targets -- -D warnings` → clean.
2. `tradebot policy check` prints `policy_version = sha256:…` (a loaded file),
   not `builtin:defaults`.
3. After provisioning: `tradebot whoami` prints the real user id and margins.
4. `tradebot status` shows the kill switch disengaged and a believed-valid
   token; `tradebot ledger` lists whatever the new home has (empty is correct
   on a fresh box — the ledger does not migrate).
5. `tradebot kill` then `tradebot status` → `ENGAGED`; `tradebot resume` to
   clear. This proves the operator's stop button works on the new host before
   anything is allowed to trade.
