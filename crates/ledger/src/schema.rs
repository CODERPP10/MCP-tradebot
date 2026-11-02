//! Embedded SQLite schema (DESIGN.md §6). Applied idempotently on every
//! [`crate::Ledger::open`] via `CREATE TABLE IF NOT EXISTS`.

pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS intents (
  ref_id            TEXT PRIMARY KEY,
  created_at        TEXT NOT NULL,
  source            TEXT NOT NULL,
  tradingsymbol     TEXT NOT NULL,
  exchange          TEXT NOT NULL,
  side              TEXT NOT NULL,
  quantity          INTEGER NOT NULL,
  order_type        TEXT NOT NULL,
  product           TEXT NOT NULL,
  limit_price       REAL NOT NULL,
  intent_hash       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_intents_hash_time ON intents(intent_hash, created_at);

CREATE TABLE IF NOT EXISTS decisions (
  ref_id            TEXT NOT NULL REFERENCES intents(ref_id),
  decided_at        TEXT NOT NULL,
  decision          TEXT NOT NULL,
  policy_version    TEXT NOT NULL,
  reasons_json      TEXT NOT NULL,
  ltp_at_decision   REAL,
  PRIMARY KEY (ref_id, decided_at)
);

CREATE TABLE IF NOT EXISTS confirmations (
  ref_id            TEXT PRIMARY KEY REFERENCES intents(ref_id),
  channel           TEXT NOT NULL,
  nonce             TEXT,
  requested_at      TEXT NOT NULL,
  resolved_at       TEXT,
  outcome           TEXT
);

CREATE TABLE IF NOT EXISTS submissions (
  ref_id            TEXT PRIMARY KEY REFERENCES intents(ref_id),
  mode              TEXT NOT NULL,
  submitting_at     TEXT NOT NULL,
  broker_order_id   TEXT,
  broker_tag        TEXT,
  accepted_at       TEXT,
  submit_error      TEXT
);

CREATE TABLE IF NOT EXISTS order_status (
  ref_id            TEXT NOT NULL REFERENCES intents(ref_id),
  observed_at       TEXT NOT NULL,
  broker_order_id   TEXT,
  status            TEXT NOT NULL,
  filled_quantity   INTEGER NOT NULL,
  pending_quantity  INTEGER NOT NULL,
  average_price     REAL,
  status_message    TEXT,
  PRIMARY KEY (ref_id, observed_at)
);

CREATE TABLE IF NOT EXISTS kill_switch (
  id                INTEGER PRIMARY KEY CHECK (id = 1),
  engaged           INTEGER NOT NULL,
  changed_at        TEXT NOT NULL,
  changed_by        TEXT NOT NULL,
  note              TEXT
);

CREATE TABLE IF NOT EXISTS operator_audit (
  at                TEXT NOT NULL,
  action            TEXT NOT NULL,
  detail            TEXT
);

CREATE TABLE IF NOT EXISTS reconciliation_divergences (
  observed_at       TEXT NOT NULL,
  kind              TEXT NOT NULL,
  detail_json       TEXT NOT NULL
);
"#;
