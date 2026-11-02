# Project status

_Fresh machine? `HANDOFF.md` has provisioning, environment and the
non-negotiables an autonomous agent must respect._

_Last updated: 2026-11-02_

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
4. **`feat/session-auth`** — _in progress (demo-first slice, off
   `feat/secret-store`)._ `TokenProvider` trait (async, `async-trait`) +
   `ManualPaste`: `login_url()`, `complete_login(request_token)` →
   `kite-client.generate_session` → seals `{access_token, issued_at,
   assumed_invalid_after}` (JSON) under `kite_access_token`. `assumed_invalid_after`
   = next 06:00 IST (chrono, fixed +05:30). `access_token()` → `Valid` while
   inside the window, else `NeedsLogin { login_url }`. `secret-store` gained
   `SharedSecretStore` (`Arc<Mutex<dyn SecretStore + Send>>`) + `shared()`.
   `tradebot login` wired (opens store, prints URL, seals token). Prompt helpers
   factored into `operator-cli/src/prompt.rs`.
5. **`feat/engine-reads`** — _in progress (demo-first slice, off `feat/session-auth`)._
   `Engine` = `KiteClient` + `Arc<dyn TokenProvider>`. Six read services
   (`positions/holdings/margins/orders/quote/order_status`), each authenticates
   via `session` (→ `EngineError::NeedsLogin` when the token is stale) then
   shapes one Kite read into a §5 view. `mcp-server`: `rmcp` 3.2 stdio server,
   `tradebot-mcp serve --paper`, six read-only tools, passphrase from
   `TRADEBOT_PASSPHRASE`. `--live` refused. `tradebot whoami` added on the prior
   branch. **⇒ Milestone A** (Claude Desktop reads the live account).
6. **`feat/ledger`** — _done._ `rusqlite` (bundled), the full §6 schema,
   `intent_hash`/duplicate detection, daily aggregates derived by query,
   kill-switch + operator audit trail.
7. **`feat/guardrails`** — _done._ All 11 §2.5 checks implemented against a
   real `AccountContext`; 31 unit tests (one pass/fail pair per check plus
   boundary cases), verified to actually fail when a check is broken.
8. **`feat/engine` (submit + reconcile)** — _done._ `Engine::submit` follows
   §2.6 steps 1–6: instrument resolution (cached), live `AccountContext`
   assembly, `guardrails::evaluate`, ledger decision/submission rows,
   `PaperExecutor` (simulated immediate fill — no live executor yet).
   `resume_confirmed` lets `telegram-confirm`/the CLI fallback resume a
   `NeedsHumanConfirm` order. `reconcile_once`/`reconcile_loop` landed
   2026-11-02 (below). `modify_order` remains out of scope.
9. **`feat/telegram-confirm`** — _done._ Bot API client (`getMe`/
   `sendMessage`+inline buttons/`getUpdates`/`answerCallbackQuery`), nonce +
   TTL, fails closed on timeout/network/wrong-chat-id. Decoupled from `engine`
   via a `ConfirmResumer` trait + `domain::ConfirmOutcome` (avoids a crate
   dependency cycle) — wired together at the `mcp-server` boot layer.
10. **`feat/mcp-server` (write)** — _done._ `place_equity_order` /
    `cancel_order` tools calling `engine::submit`/`cancel`; `Confirmer::run()`
    runs as a background task when Telegram credentials are sealed. `--live`
    still refused.
11. **`feat/operator-cli`** — _done._ `kill` / `resume` / `confirm` /
    `reject` / `status` / `ledger` / `reconcile` all wired to the
    ledger/engine. `main`'s `match` is exhaustive with no "not implemented"
    fallback, so a new subcommand without an arm is a compile error.
12. **`integration/mvp`** — _test done, merge pending._ The end-to-end
    paper-mode test exists (`crates/mcp-server/tests/end_to_end_paper.rs`);
    what remains is opening the PR chain back to `main`.

Demo-first slice reorders this: 5 (reads) landed before ledger/guardrails so
Milestone A ships first. 8 depends on 6/7; 9–11 depend on 8.

13. **Brainstorm — research agent layer (not scheduled).** An idea floated
    alongside the ledger/guardrails/engine-submit/telegram-confirm push: a
    layer that pulls market news and macro-indicator context for the trade
    horizon (the next few weeks) and surfaces it to the LLM ahead of order
    construction — earnings calendars, rate-decision dates, sector-level
    sentiment, that kind of thing. Not designed yet; open questions include
    which sources are trustworthy enough to script against, how staleness is
    surfaced, and how strongly this should shape agent behavior versus just
    being read-only background. The one settled constraint: this is a
    **read/advisory layer only** — it feeds context the agent sees before
    producing an `OrderIntent`, the same as any other read tool. It gets no
    new path to Kite, no ability to loosen a cap, bypass
    `guardrails::evaluate`, or auto-confirm an order. The guardrail chain
    stays the only choke point regardless of what informed the request.

## 2026-09-12 session: ledger → guardrails → engine-submit → telegram-confirm

Built the whole safety chain (roadmap items 6–11 above) in one push on top of
`feat/engine-reads`, verified stage by stage (`cargo build/test/clippy -D
warnings` per crate, then the same across the whole workspace — 102 tests,
0 warnings, `fmt --check` clean).

**New/changed since the last update:**
- `domain::ConfirmOutcome` — shared between `engine` and `telegram-confirm`
  without either crate depending on the other (they're wired together at the
  `mcp-server` boot layer via a `ConfirmResumer` trait), avoiding a dependency
  cycle back to `engine`, which owns the `pending_confirm` row `telegram-confirm`
  polls for.
- `Engine::with_submit` (full construction, needs `Ledger` + `Policy`) alongside
  the existing `Engine::new` (read-only, unaffected — `tests/reads.rs` needed
  no changes). `Engine::with_fixed_clock` is a test-only hook so the
  session-hours/quote-age guardrail checks don't flake on the real wall clock.
- `mcp-server` now exposes 8 tools (6 read + `place_equity_order` +
  `cancel_order`) and boots a background `Confirmer::run()` task when Telegram
  credentials are sealed — `serve --paper` prints a warning and falls back to
  CLI-only confirmation (`tradebot confirm`/`reject`) if they aren't.
- `operator-cli` gained `secrets put <key>` / `secrets list` — seals an
  additional record (e.g. Telegram creds) into an *existing* store without
  the destructive re-`init` that would lose the sealed Kite key/secret/token.
- `crates/mcp-server/examples/smoke_confirm.rs` — a standalone manual-test
  binary that builds the same real `Engine`+`Confirmer` wiring as `serve`,
  submits one tiny paper order, and prints the Telegram confirm outcome
  directly (there's no CLI surface yet to query a `ref_id`'s resolution after
  the fact — see gaps below). Run via
  `cargo run -p mcp-server --example smoke_confirm -- <SYMBOL>`.

**Live-verified this session:** `tradebot secrets put`, `tradebot login`
(after finding and working around a Windows console QuickEdit-mode / stale
console-mode-after-`rpassword` issue — closing and reopening the PowerShell
window fixed it), `tradebot whoami`, and the `smoke_confirm` example's
Telegram `getMe` health check all worked against the user's real bot
(`@pp_mcp_zerodha_bot`) and real Kite session.

**Blocked at the last step, then fixed:** `smoke_confirm` failed fetching the
INFY quote with `PermissionException: Insufficient permission for that call`
— the pre-existing, already-tracked [[kite-quote-permission]] gap (the user's
Kite Connect app lacks the market-data add-on). Fixed same session:
`engine::submit`'s `account_context()` now catches that specific
`KiteError::Rejected{code: "PermissionException", ..}` and falls back to the
symbol's `last_price` from `positions` (already fetched) or, failing that,
`holdings` (one extra Kite call, only made when needed) — treated as a real
(if non-live) price that still goes through the full price-band/cap/confirm
checks, not a bypass. If the symbol isn't held anywhere, `ltp` stays `None`
and guardrails rejects `QUOTE_STALE` as before — the check is never silently
skipped. Two new engine tests cover both branches
(`quote_permission_denied_falls_back_to_held_position_last_price`,
`..._with_no_held_position_or_holding_fails_closed_as_quote_stale`); workspace
is back to green (104 tests, 0 clippy warnings, fmt clean). **Not yet
re-verified live** — the user hadn't re-run `smoke_confirm` against their
real account as of this writing, so the Telegram round-trip (bot
reachability, nonce, buttons, callback handling) is still only verified via
`cargo test -p telegram-confirm`'s mocked end-to-end test, not a real tap in
Telegram.

**Also added this session: automatic Telegram relogin.** `mcp-server` now
polls the Kite token status every 5 minutes (`boot.rs`'s `LOGIN_CHECK_INTERVAL`)
and, on `NeedsLogin`, has the same bot DM the login URL to the allowlisted
chat; replying with the `request_token` (bare value or the whole pasted
redirect URL — `telegram_confirm::extract_request_token` accepts either)
completes the login automatically via `session::ManualPaste::complete_login`,
decoupled the same way order confirmation is (a `LoginResumer` trait,
`mcp-server`'s `KiteLoginResumer` implements it). This shares `Confirmer`'s
single `getUpdates` poll loop rather than running a second one — Telegram's
long-poll offset is a single shared queue per bot token, so two independent
pollers would nondeterministically steal each other's updates; see the
`telegram-confirm::confirmer` module docs. `tradebot login` (CLI paste) still
works unchanged as a manual fallback. 7 new tests
(`crates/telegram-confirm/tests/confirmer.rs` + `src/login.rs`) cover
dedup-while-awaiting, wrong-chat rejection, unsolicited-message rejection,
and retry-after-failure.

**Known gaps to close next:**
1. Re-run `cargo run -p mcp-server --example smoke_confirm -- INFY` (only
   works if INFY — or the symbol you pass — is currently held; otherwise pick
   a held symbol, or buy the market-data add-on for a true live quote) to get
   the first real end-to-end Telegram confirmation.
2. `policy.toml` at the repo root is still the DESIGN.md placeholder — fine
   for paper mode, must be replaced with real numbers before `--live` is ever
   considered (which also requires a live executor that doesn't exist yet).

## 2026-10-26 session: `tradebot status` + `tradebot ledger`

Closed the old gap 2 — there is now a way to see what the bot did without
`smoke_confirm`'s printout.

- `ledger` gained a read-only operator API: `kill_switch()` (the whole row,
  not just the boolean), `last_divergence()`, and an `OrderRow` projection
  (`order(ref_id)` / `orders_today()` / `recent_orders(limit)`) that joins each
  intent to its **latest** decision, its confirmation, its submission and its
  latest observed broker status. Nothing in the write path uses it.
- `tradebot status [--home] [--no-session]` — token belief (read from the
  sealed store, no Kite call; `whoami` is still the live check), kill-switch
  state with who/when, today's aggregates, pending confirmations, and the
  last reconciliation divergence. `--no-session` skips the passphrase prompt.
- `tradebot ledger [--today] [--pending] [--ref <id>] [--limit N]` — pure
  local SQLite reads, no passphrase. Bare invocation lists the 20 most recent
  orders; `--ref` prints one order's whole lifecycle including the guardrail
  rejection reasons.
- 6 new ledger tests + 3 CLI formatting tests. Workspace: 114 tests, 0 clippy
  warnings, `fmt --check` clean.

Verified by hand against the real `.tradebot` home: the one order in it (the
`smoke_confirm` INFY attempt) shows up as `REJECTED` with both
`INSUFFICIENT_MARGIN` and `MARKET_CLOSED` reasons.

**Stale build-cache gotcha:** `cargo test --workspace` failed once in
`guardrails::policy::tests::repo_policy_toml_parses_and_validates` with a
`CARGO_MANIFEST_DIR` pointing at `Projects\MCP-tradebot` (missing the `MCP\`
path component) — a cached artifact from before the repo moved. `cargo clean
-p guardrails` fixed it; the test itself is fine. Worth remembering if a
path-dependent test fails inexplicably.

## 2026-11-02 session: reconciliation, policy provenance, end-to-end test

Three parallel pushes closed the last of roadmap items 8, 11 and 12. Workspace
after all of it: **150 tests, 0 clippy warnings (`-D warnings`), `fmt --check`
clean.**

### Reconciliation (DESIGN.md §4) — roadmap item 8's deferred half
- `ledger`: `open_submissions()` (non-paper rows whose latest status is not
  terminal), `submission_broker_keys()`, `record_divergence_once()`,
  `list_divergences(limit)`, plus `MODE_PAPER` / `TERMINAL_STATUSES` /
  `is_terminal_status()`. The SQL's terminal `IN (...)` list is generated from
  `TERMINAL_STATUSES` so the query and the predicate cannot drift.
- `engine::reconcile_once()` -> `ReconcileReport` (counts + the divergences
  recorded *this* pass) and `engine::reconcile_loop(interval)`. It polls open
  orders to terminal state, re-links a NULL-broker-id submission by its
  `tradebot:<ref_id>` tag (§8 risk 9), marks a live `submitting` row with no
  broker order as `submit_failed`, and flags `unknown_broker_order` /
  `missing_broker_order` divergences. It **never** re-sends: the test suite
  mounts POST/PUT/DELETE mocks with `.expect(0)` so any resend fails the test.
- **Paper orders are excluded by construction.** `PAPER-<ref_id>` ids never
  appear in Kite's order book, so including them would turn every paper fill
  into a false divergence; `open_submissions` filters `mode <> 'paper'` and a
  test asserts paper submissions produce zero divergences.
- Divergences are deduplicated on byte-identical `(kind, detail_json)` — a
  foreign broker order is divergent on *every* pass, and appending it each
  minute would add thousands of rows a day. Consequence: a second
  `tradebot reconcile` shows an empty "new divergences" list, which is why it
  also prints `list_divergences()` for the standing picture.
- `positions`/`margins` are deliberately not fetched in the pass — nothing
  reads them there, and `submit` already pulls them live where they matter.
- Wiring: `tradebot reconcile` (new `crates/operator-cli/src/reconcile.rs`)
  runs one pass by hand; `tradebot-mcp` spawns `reconcile_loop` at boot with
  `RECONCILE_INTERVAL = 60s`. The loop's first pass is immediate, which is the
  "on startup" half of §4; the "before every order decision" half was already
  covered by `submit`'s live `AccountContext`.
- Logging is `eprintln!`, matching the rest of the workspace — there is no
  `tracing` subscriber anywhere, so `tracing` calls would have been invisible.

### Policy provenance — `decisions.policy_version` is real now
`Policy::load` hashes the file (SHA-256 over the text with CRLF normalised to
LF, so a Windows checkout and a Unix one agree) and `Policy::version()`
returns `sha256:<first 16 hex>`; `Policy::default()` returns the sentinel
`builtin:defaults`, deliberately not hash-shaped so a decision made under
compiled-in defaults can never be mistaken for one made under an operator
file. `engine`'s `record_decision` writes it, replacing the old
`POLICY_VERSION_UNTRACKED` placeholder, and `tradebot policy check` prints
`policy_version = ...` (currently `sha256:9545aa3584de880f`).

`Policy`'s `PartialEq` is now hand-written and **excludes** `version`:
equality there means "same effective guardrail numbers", not "same
provenance", so `repo_policy_toml_parses_and_validates` can still assert the
repo file matches the built-in defaults numerically while asserting the
versions differ. The impl destructures with `version: _`, so a future field
added without a decision about equality is a compile error.

### End-to-end paper-mode test — roadmap item 12
`crates/mcp-server/tests/end_to_end_paper.rs` drives the **real MCP tool
layer** over `tokio::io::duplex` with an `rmcp` client, so tests enter by tool
*name* (`tools/call`), which is precisely the invariant worth guarding: no
path from a tool to an order that skips `engine::submit` ->
`guardrails::evaluate`. `mcp-server` is bin-only, so the test includes the
handler with `#[path = "../src/server.rs"] mod server;` — no `[lib]` target
added, and `server.rs` itself is unchanged. Kite is wiremock-mocked with **no
mock for `POST /orders/:variety`**, so touching a mutating endpoint in paper
mode fails the test. Seven scenarios: approved-and-journalled (asserted field
by field across all five §6 tables), cap rejection with no `submissions` row,
the §2.7 pending-confirmation detour (wrapped in a timeout so a *blocking*
implementation fails rather than hangs) and its approval/rejection halves,
kill switch, read tools, and set-equality on the eight designed tools.

**Test-authoring caveat found and documented on `with_fixed_clock`:** ledger
rows are stamped with the real wall clock, so with a past fixed clock
`seconds_since_last_order` goes negative and clamps to 0 — tripping the
rate-limit check. A fixed-clock test can submit only one order per ledger;
give each scenario a fresh `Ledger`.

**Build-cache note (second sighting):** concurrent cargo invocations on
Windows log `did not finalize incremental compilation session directory ...
(os error 32)`. Harmless lock contention, not a build failure.

## Immediate blockers / setup needed

- Rust toolchain installed (rustup, stable `1.98.1`, pinned in
  `rust-toolchain.toml`). Skeleton verified: `cargo build` / `clippy -D warnings`
  / `test` (11 pass) / `fmt --check` all green; `tradebot policy check` works.
- Open the PR for `feat/workspace-skeleton` → `main` and set `main` as the
  GitHub default branch + branch protection (gh CLI not installed).
- BFG history scrub (owner: user, later).
- Kite market-data add-on not subscribed — see the quote-permission gap above.
