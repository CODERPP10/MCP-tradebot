# MCP Tradebot — Design (Rust rewrite)

Status: **design agreed, no implementation yet**
Last updated: 2026-09-01

An MCP server that lets an LLM agent (Claude Desktop, Hermes, Claude Code, …) place
orders on a personal Zerodha (Kite Connect) account, with guardrails that are
enforced structurally — independent of whatever the LLM decides.

The existing TypeScript prototype (`trade.ts`, `index.ts`, `mcp-trader.ts`) is the
reference for behaviour only. It will be replaced, not ported.

> **"Faster in Rust" is a non-goal.** Order latency is dominated by Kite's network
> round-trip and rate limits (~3 req/s, 200 orders/min, 3000/day). Rust is chosen
> for a single audited static binary, no npm supply chain, memory safety, and
> strong types for money / quantity / side.

---

## 1. v1 scope (locked)

| Dimension | v1 value |
|---|---|
| Exchange | NSE only |
| Product | CNC (delivery) only |
| Variety | `regular` only (no AMO / CO / BO) |
| Order type | LIMIT only — price required, within ±`limit_price_band_pct` of LTP |
| Validity | DAY only |
| Instrument | NSE `EQ`-series equity, resolved against the daily instrument dump |
| Sides | BUY / SELL, integer share quantity |
| Token refresh | Manual each morning (`tradebot login`), behind a `TokenProvider` trait |
| Human confirm | Telegram bot (Approve/Reject), CLI fallback, fail-closed |
| Execution | Paper-first — simulated fills until `--live` is passed |
| Secrets at rest | Passphrase-sealed store (Argon2id + XChaCha20-Poly1305), `SecretStore` trait |

Explicitly **rejected by the guardrail allowlist in v1** (added deliberately later):
MARKET orders, MIS/intraday, BSE, F&O / derivatives, GTT, AMO, bracket/cover orders.

Deferred to v1.1: `modify_order`, KMS-backed secret store, automated (scheduled)
token refresh, Kite postback webhooks.

---

## 2. Module boundaries (cargo workspace)

One crate per boundary. Dependency arrows point *downward* only.

```
                       operator-cli ──┐
                                      │
  mcp-server ──► engine ──► guardrails├──► ledger
                    │            │    │
                    ├──► session ─────┤
                    │       │         │
                    └──► kite-client  │
                            │         │
                        secret-store ◄┘
```

### 2.1 `kite-client`
Pure Kite Connect REST client (+ optional WS ticker later). **No business logic.**

- Auth header `Authorization: token <api_key>:<access_token>`.
- Session checksum `SHA256(api_key + request_token + api_secret)`.
- Client-side rate limiter (token bucket) sized under Kite's published limits.
- Typed request/response models; a normalised error taxonomy
  (`Auth`, `RateLimited`, `NetworkTimeout`, `Rejected{code,message}`, `Unknown`).
- Methods: `generate_session`, `place_order`, `modify_order`, `cancel_order`,
  `orders`, `order_history`, `positions`, `holdings`, `margins`, `quote`, `ltp`,
  `profile`, `instruments`.
- **Never logs** secrets, tokens, or full request bodies.
- Tested against recorded HTTP fixtures (no live calls in CI).

### 2.2 `secret-store`
Sealed at-rest storage for all secrets.

- `trait SecretStore { fn get(&self, key) -> Result<Secret>; fn put(&self, key, Secret); fn list_keys(); }`
- v1 impl `SealedFileStore`: single file, per-record XChaCha20-Poly1305, key
  derived from an operator passphrase via Argon2id. Key material lives in memory
  only (zeroized on drop), never written to disk.
- Passphrase is required **on every process start** (server and CLI) — a crash
  does not auto-recover unattended. This is intentional for v1.
- Stored records: `kite_api_key`, `kite_api_secret`, `kite_access_token` (+ meta),
  `telegram_bot_token`, `telegram_chat_id`.
- Later impl `KmsStore` (AWS/GCP KMS envelope encryption) slots in behind the trait.

### 2.3 `session`
Access-token lifecycle. Kite tokens are invalidated at the start of each trading
day (~06:00 IST); there is **no supported unattended login**.

- `trait TokenProvider { async fn access_token(&self) -> Result<AccessToken, NeedsLogin>; }`
- v1 impl `ManualPaste`:
  - `login_url()` → the Kite connect URL for `api_key`.
  - `complete_login(request_token)` → calls `kite-client.generate_session`,
    seals the resulting `access_token` + `issued_at` + `assumed_invalid_after`
    into `secret-store`.
  - `access_token()` → returns the sealed token if still within its validity
    window, else `NeedsLogin { login_url }`.
- Validity is a local best-effort estimate; the engine treats a live `Auth` error
  from `kite-client` as authoritative and surfaces `NeedsLogin`.
- Future impl `ScheduledAutomated` (separate OS task, **not** agent-reachable,
  **not** part of the MCP binary) can replace `ManualPaste` with zero downstream
  change.

### 2.4 `ledger`
Append-only journal. SQLite (WAL). Local source of truth for *what the bot did*;
**not** the source of truth for positions or order state (those are reconciled
live — see §4).

- Every intent, guardrail decision, confirmation outcome, submission, broker
  order id, and terminal status is a row.
- Daily counters (order count, gross notional, capital deployed) are **derived by
  query**, never stored as mutable totals — avoids drift.
- Idempotency: `intent_hash = H(tradingsymbol, side, qty, order_type, price_bucket, epoch_window)`
  with a uniqueness/lookup index for rapid-fire duplicate rejection.
- Schema in §6.

### 2.5 `guardrails`
Deterministic, synchronous **policy engine**. The security-critical core.

- `fn evaluate(intent: &OrderIntent, ctx: &AccountContext, policy: &Policy) -> Decision`
- **No network calls. No dependency on `mcp-server`.** Compiles without the tool
  layer even existing — so there is no code path from a tool to Kite that skips it.
- Reads `Policy` from a TOML file owned by the operator user, **not writable by
  the server process**; the server exposes no policy-editing tool. Policy version
  hash is recorded with every decision.
- `AccountContext` is assembled by `engine` from *live* Kite data (positions,
  margins, LTP, open orders) plus *ledger*-derived daily aggregates.
- Checks (all produce structured `Reason` values, never a bare bool):
  1. Kill switch engaged (DB row or flag file) → reject everything mutating.
  2. Instrument allowlist: NSE `EQ` equity, resolvable in today's instrument dump;
     optional explicit `symbol_allowlist`.
  3. Order shape: exchange=NSE, product=CNC, variety=regular, type=LIMIT,
     validity=DAY — anything else rejected.
  4. Price sanity: `limit_price` within ±`limit_price_band_pct` of LTP;
     reject if LTP stale (> `max_quote_age_secs`) or instrument at circuit limit.
  5. Per-order cap: `qty * limit_price <= max_order_value`.
  6. Per-symbol position cap: projected position value `<= max_position_value_per_symbol`.
  7. Daily aggregates (from ledger): `orders_today < max_orders_per_day`,
     `gross_notional_today + this <= max_gross_notional_per_day`,
     `net_new_capital_today + this <= max_new_capital_per_day`.
  8. Margin: projected requirement `<= available_margin - margin_buffer`.
  9. Trading session: within 09:15–15:30 IST, weekday, not an NSE holiday.
  10. Rate: `now - last_order_ts >= min_seconds_between_orders`.
  11. Duplicate: matching `intent_hash` within the idempotency window → reject.
- `enum Decision { Approved, Rejected { reasons: Vec<Reason> }, NeedsHumanConfirm { reasons: Vec<Reason> } }`
- `NeedsHumanConfirm` is returned when an approved order exceeds
  `confirm_above_value` (or when `confirm_all = true`).

### 2.6 `engine`
The single choke point. Orchestrates everything; the only crate that calls
`kite-client` mutating methods.

- `async fn submit(intent: OrderIntent) -> SubmitOutcome`
  1. Resolve + validate instrument against instrument cache.
  2. Reconcile `AccountContext` live from Kite (positions, margins, LTP, orders).
  3. `guardrails::evaluate(...)`.
  4. `Rejected` → write decision to ledger, return structured rejection.
  5. `NeedsHumanConfirm` → write `pending_confirm` row, hand off to
     `telegram-confirm`, return `SubmitOutcome::Pending { ref_id }` **immediately**.
  6. `Approved` → write `submitting` row **before** the Kite call → call
     `kite-client.place_order` (or paper executor) with
     `tag = "tradebot:<ledger_id>"` → record broker order id → return
     `SubmitOutcome::Accepted { order_id, ref_id }`.
- Confirmed pending orders resume at step 6 via a callback from `telegram-confirm`.
- A background reconciliation task (§4) polls open orders to terminal state and
  updates the ledger.
- **Never auto-retries a mutating call.** "Accepted by Kite, final state unknown"
  is a first-class outcome left for reconciliation.
- Read services: `positions()`, `holdings()`, `margins()`, `quote()`,
  `order_status()`, `orders()` — pass-through to `kite-client` with light shaping.
- Paper mode: `PaperExecutor` simulates immediate/partial fills at or near the
  limit price, writes the same ledger rows, touches no Kite mutating endpoint.

### 2.7 `telegram-confirm`
Out-of-band human approval.

- Long-polls the Telegram Bot API (no inbound port needed).
- On `pending_confirm`: sends a formatted order summary to `telegram_chat_id`
  with inline **Approve** / **Reject** buttons carrying a per-request nonce.
- Accepts a callback **only** from the allowlisted `chat_id` with a matching,
  unexpired nonce. Marks the ledger row and notifies `engine`.
- TTL `confirm_ttl_secs` (default 120). On timeout, network failure, or bot
  unreachable → **fail closed** (reject), ledger row `confirm_timeout`.
- Startup health check: verify `getMe` succeeds, else refuse to start in a mode
  that can produce `NeedsHumanConfirm`.
- CLI fallback: `tradebot confirm <ref_id>` / `tradebot reject <ref_id>` writes
  the same ledger outcome.

### 2.8 `mcp-server`
The MCP tool layer. `rmcp`, stdio transport. **Thin** — schema in / schema out,
no trading logic, no direct `kite-client` use.

- Translates each tool call into an `OrderIntent` or a read query, calls `engine`,
  formats the response.
- Tool schemas in §5.
- A `place_equity_order` that hits `NeedsHumanConfirm` returns
  `{ status: "pending_confirmation", ref_id }` — it does **not** block. The agent
  polls `get_order_status`.

### 2.9 `operator-cli`
Human control, out-of-band from the agent.

- `tradebot init` — create sealed store, prompt for api_key/secret, telegram token/chat_id.
- `tradebot login` — print login URL, accept pasted `request_token`, seal access token.
- `tradebot status` — token validity, kill-switch state, today's ledger aggregates, last reconciliation.
- `tradebot kill` / `tradebot resume` — toggle the kill switch (audited).
- `tradebot confirm <ref_id>` / `tradebot reject <ref_id>` — confirm fallback.
- `tradebot ledger [--today] [--pending]` — inspect the journal.
- `tradebot policy check` — validate the policy TOML, print the effective values.
- `tradebot reconcile` — force a reconciliation pass, print divergences.

---

## 3. Where guardrails are enforced

- `guardrails` is a **separate crate that does not depend on `mcp-server`**. There
  is no compile path for a tool to reach a Kite order endpoint without
  `engine::submit()`, and `submit()` unconditionally calls `guardrails::evaluate()`.
- The LLM only ever produces a **schema-validated `OrderIntent`**. There is no
  "raw passthrough" tool.
- `Policy` comes from a file the server process cannot write; there is no
  policy-editing tool.
- Kill switch is checked first inside `evaluate()`, and again as a hard gate in
  `engine::submit()`.
- Defence in depth: set conservative caps in the Kite dashboard too, and fund the
  account with only the capital you are willing to expose.

---

## 4. State: persist vs. reconcile

**Persisted locally (authoritative here):**
- Sealed secrets: api key/secret, access token + meta, telegram token/chat_id.
- Ledger: intents, decisions, confirmations, submissions, broker ids, statuses.
- Policy TOML + a recorded version hash per decision.
- Kill-switch state + operator action audit log.
- Idempotency index.
- Instrument dump cache (refreshed daily).

**Reconciled live from Kite (local copy is never trusted as truth):**
- Positions, holdings, available margin / funds.
- Open-order status and fills.
- LTP / quotes.
- Profile / account info.

**Reconciliation loop:** on startup and before every order decision, pull
`orders` + `positions` + `margins` and match against the ledger by broker order id
and by `tag = "tradebot:<ledger_id>"`. Flag divergences:
- broker order with no ledger row → something else is trading the account;
- ledger `submitting` with no matching broker order → silent submit failure,
  mark `submit_failed`, do **not** auto-resend.

Derived daily aggregates are always recomputed from the ledger, never cached as
mutable counters.

---

## 5. MCP tool schemas (v1)

All money in rupees, all quantities in whole shares. Timestamps ISO-8601 IST.

### Write

```
place_equity_order
  in:  { tradingsymbol: string,        # e.g. "INFY" (NSE EQ series)
         side: "BUY" | "SELL",
         quantity: integer > 0,
         limit_price: number > 0 }
  out: { status: "accepted" | "pending_confirmation" | "rejected",
         ref_id: string,               # ledger id, always present
         order_id: string | null,      # broker id when accepted
         reasons: [ { code: string, detail: string } ] }   # when rejected/pending

cancel_order
  in:  { order_id: string }
  out: { status: "accepted" | "rejected", reasons: [...] }
```

### Read

```
get_positions   -> { net: [ { tradingsymbol, quantity, average_price, last_price, pnl } ], day: [...] }
get_holdings    -> { holdings: [ { tradingsymbol, quantity, average_price, last_price, pnl } ] }
get_margins     -> { equity: { available: number, utilised: number, net: number } }
get_quote       in { tradingsymbol } -> { last_price, ohlc, timestamp, tradable: bool, at_circuit_limit: bool }
get_order_status in { order_id } -> { status, filled_quantity, pending_quantity, average_price, status_message, history: [...] }
list_orders     -> { orders: [ { order_id, tradingsymbol, side, quantity, filled_quantity, limit_price, status, ref_id } ] }
```

### Error / rejection codes (structured `reasons`)

`KILL_SWITCH`, `INSTRUMENT_NOT_ALLOWED`, `INSTRUMENT_UNKNOWN`, `ORDER_SHAPE`,
`PRICE_OUT_OF_BAND`, `QUOTE_STALE`, `AT_CIRCUIT_LIMIT`, `ORDER_VALUE_CAP`,
`SYMBOL_POSITION_CAP`, `DAILY_ORDER_COUNT`, `DAILY_NOTIONAL_CAP`,
`DAILY_CAPITAL_CAP`, `INSUFFICIENT_MARGIN`, `MARKET_CLOSED`, `RATE_LIMITED`,
`DUPLICATE_ORDER`, `NEEDS_LOGIN`, `CONFIRM_TIMEOUT`, `CONFIRM_REJECTED`,
`BROKER_REJECTED`, `RECONCILIATION_DIVERGENCE`.

---

## 6. Ledger schema (SQLite, WAL)

```sql
CREATE TABLE intents (
  ref_id            TEXT PRIMARY KEY,          -- ULID
  created_at        TEXT NOT NULL,             -- ISO-8601 IST
  source            TEXT NOT NULL,             -- "mcp" | "cli"
  tradingsymbol     TEXT NOT NULL,
  exchange          TEXT NOT NULL,             -- "NSE"
  side              TEXT NOT NULL,             -- "BUY" | "SELL"
  quantity          INTEGER NOT NULL,
  order_type        TEXT NOT NULL,             -- "LIMIT"
  product           TEXT NOT NULL,             -- "CNC"
  limit_price       REAL NOT NULL,
  intent_hash       TEXT NOT NULL
);
CREATE INDEX idx_intents_hash_time ON intents(intent_hash, created_at);

CREATE TABLE decisions (
  ref_id            TEXT NOT NULL REFERENCES intents(ref_id),
  decided_at        TEXT NOT NULL,
  decision          TEXT NOT NULL,             -- "APPROVED" | "REJECTED" | "NEEDS_CONFIRM"
  policy_version     TEXT NOT NULL,
  reasons_json      TEXT NOT NULL,             -- [] when approved
  ltp_at_decision   REAL,
  PRIMARY KEY (ref_id, decided_at)
);

CREATE TABLE confirmations (
  ref_id            TEXT PRIMARY KEY REFERENCES intents(ref_id),
  channel           TEXT NOT NULL,             -- "telegram" | "cli"
  nonce             TEXT,
  requested_at      TEXT NOT NULL,
  resolved_at       TEXT,
  outcome           TEXT                       -- "APPROVED" | "REJECTED" | "TIMEOUT"
);

CREATE TABLE submissions (
  ref_id            TEXT PRIMARY KEY REFERENCES intents(ref_id),
  mode              TEXT NOT NULL,             -- "live" | "paper"
  submitting_at     TEXT NOT NULL,             -- written BEFORE the broker call
  broker_order_id   TEXT,
  broker_tag        TEXT,                      -- "tradebot:<ref_id>"
  accepted_at       TEXT,
  submit_error      TEXT
);

CREATE TABLE order_status (                    -- updated by reconciliation
  ref_id            TEXT NOT NULL REFERENCES intents(ref_id),
  observed_at       TEXT NOT NULL,
  broker_order_id   TEXT,
  status            TEXT NOT NULL,             -- OPEN | COMPLETE | CANCELLED | REJECTED | ...
  filled_quantity   INTEGER NOT NULL,
  pending_quantity  INTEGER NOT NULL,
  average_price     REAL,
  status_message    TEXT,
  PRIMARY KEY (ref_id, observed_at)
);

CREATE TABLE kill_switch (
  id                INTEGER PRIMARY KEY CHECK (id = 1),
  engaged           INTEGER NOT NULL,
  changed_at        TEXT NOT NULL,
  changed_by        TEXT NOT NULL,
  note              TEXT
);

CREATE TABLE operator_audit (
  at                TEXT NOT NULL,
  action            TEXT NOT NULL,             -- "kill" | "resume" | "login" | "confirm" | "reject" | "policy_reload"
  detail            TEXT
);

CREATE TABLE reconciliation_divergences (
  observed_at       TEXT NOT NULL,
  kind              TEXT NOT NULL,             -- "unknown_broker_order" | "missing_broker_order" | "quantity_mismatch"
  detail_json       TEXT NOT NULL
);
```

Daily aggregate query (illustrative): join `submissions` + latest `order_status`
for `date(submitting_at) = <today IST>`, sum `filled_quantity * average_price`
for notional, count distinct `ref_id` for order count.

---

## 7. Guardrail policy schema (`policy.toml`)

Operator-owned, not writable by the server process. Values below are placeholders
— set real numbers before going `--live`.

```toml
[kill_switch]
# runtime state lives in the DB; this is a hard file-level override
force_engaged = false

[instruments]
exchange           = "NSE"
series             = ["EQ"]
symbol_allowlist   = []          # empty = any resolvable NSE EQ symbol
require_in_instrument_dump = true

[order_shape]
product   = "CNC"
variety   = "regular"
order_type = "LIMIT"
validity  = "DAY"

[price]
limit_price_band_pct = 3.0       # limit must be within ±3% of LTP
max_quote_age_secs   = 15
reject_at_circuit_limit = true

[caps]
max_order_value                = 25000
max_position_value_per_symbol  = 50000
max_orders_per_day             = 10
max_gross_notional_per_day     = 100000
max_new_capital_per_day        = 50000
min_seconds_between_orders     = 30

[margin]
buffer = 5000

[session]
timezone      = "Asia/Kolkata"
open          = "09:15"
close         = "15:30"
holidays_file = "nse_holidays_2026.txt"

[idempotency]
window_secs   = 300
price_bucket_pct = 0.5

[confirmation]
confirm_all         = true       # v1: confirm every order
confirm_above_value = 0
confirm_ttl_secs    = 120
```

---

## 8. Risks

1. **Kite daily token expiry — no supported unattended login.** Manual
   `tradebot login` each morning is the v1 answer. Automating it later (Selenium or
   HTTP replay) means storing user id + password + TOTP seed, which is
   **full-account access** and structurally bypasses every guardrail here — the
   guardrails only bind the API path. If automated, it must run as separate OS
   infra, never as an agent-reachable tool, and the manual path must remain as a
   fallback (Zerodha changes their login flow / adds captcha periodically).
2. **LLM placing real-money orders is inherently high-risk.** Prompt injection (a
   web page or data feed the agent reads could instruct it to drain the account),
   hallucinated symbols, unit confusion (shares vs lots vs rupees). Guardrails
   mitigate, they do not eliminate. Run paper → tiny caps → confirm-every-order
   for weeks.
3. **MCP stdio has no auth and runs with your user's privileges.** The security
   boundary is "who can spawn the binary and unlock the sealed store", not the
   protocol. Any local agent pointed at it can trade within the caps.
4. **Telegram is on the safety-critical path and is not strong auth** (SIM swap,
   Telegram account takeover). Fail-closed is correct but means the bot silently
   stops trading when Telegram/network is down. Keep the CLI fallback and a
   startup health check. Do not raise caps assuming Telegram is a hardware key.
5. **Passphrase on every start** means a crash mid-session stays down until you
   re-enter it. Intentional for v1; revisit with a KMS-backed store if resilience
   is wanted.
6. **Reconciliation races.** Positions and prices move between the context pull
   and the submit; partial fills; "accepted by Kite, final state unknown". The
   engine degrades gracefully and never auto-retries a mutating call.
7. **Regulatory / broker ToS.** Automated retail order placement via Kite Connect
   sits in a grey area under SEBI's algo-trading framework. Check your obligations
   before going live.
8. **Symbol ambiguity / instrument drift.** `INFY` vs `INFY-BE`, token changes,
   corporate actions. Force explicit `tradingsymbol`, validate against the daily
   instrument dump, reject anything unresolved.
9. **Crash between ledger write and broker call.** Mitigated by writing the
   `submitting` row *before* the Kite call so reconciliation can recover; residual
   risk if the process dies after Kite accepts but before the broker id is
   recorded — the `tag` lets reconciliation re-link it.

---

## 9. Branch plan

`main` is protected and always builds.

1. `feat/workspace-skeleton` — cargo workspace, crate stubs, CI (fmt / clippy /
   test), `policy.toml` schema + `tradebot policy check`, domain types
   (`Money`, `Quantity`, `Side`, `OrderIntent`, `Decision`, `Reason`).
2. `feat/kite-client` — REST client + typed models + fixture-based tests.
3. `feat/secret-store` — `SealedFileStore`, `tradebot init`.
4. `feat/session-auth` — `TokenProvider` / `ManualPaste`, `tradebot login`.
5. `feat/ledger` — SQLite schema, migrations, append/query API, aggregate queries.
6. `feat/guardrails` — policy engine + exhaustive unit tests (heaviest coverage).
7. `feat/engine` — orchestration, reconciliation loop, `PaperExecutor`.
8. `feat/telegram-confirm` — bot long-poll, inline buttons, nonce/TTL, fail-closed.
9. `feat/mcp-server` — `rmcp` stdio server, tool schemas, Claude Desktop config
   snippet.
10. `feat/operator-cli` — `status` / `kill` / `resume` / `ledger` / `reconcile` /
    `confirm` / `reject`.
11. `integration/mvp` — merge all of the above, end-to-end paper-mode test with
    Claude Desktop, then → `main`.

3–6 can proceed in parallel after 1. 7 depends on 2/4/5/6. 8–10 depend on 7.

---

## 10. Claude Desktop integration (end state)

`claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "tradebot": {
      "command": "/opt/tradebot/bin/tradebot",
      "args": ["serve", "--paper"],
      "env": { "TRADEBOT_HOME": "/opt/tradebot" }
    }
  }
}
```

The passphrase is **not** in the config. The server reads it from a prompt on
first start (interactive) or from a systemd credential / `TRADEBOT_PASSPHRASE_FD`
when run as a service. Going live is a deliberate change of `--paper` to `--live`
plus a real `policy.toml`.
