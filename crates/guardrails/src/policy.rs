//! Operator-owned guardrail policy (DESIGN.md §7).
//!
//! Loaded from a TOML file the server process cannot write. The struct mirrors
//! `policy.toml` at the repo root. Every field is required in the file — there
//! are no silent defaults for a live policy — but [`Policy::default`] provides
//! the DESIGN.md placeholder values for tests and `--help` output.
//!
//! Every policy carries a [`Policy::version`] string — a content hash of the
//! file it was loaded from — which `engine` records in `decisions.policy_version`
//! so a decision can later be tied back to the exact guardrail numbers that
//! produced it (DESIGN.md §2.5, §4).

use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("cannot read policy file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid policy TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("policy is not internally consistent: {0}")]
    Inconsistent(String),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub kill_switch: KillSwitch,
    pub instruments: Instruments,
    pub order_shape: OrderShape,
    pub price: Price,
    pub caps: Caps,
    pub margin: Margin,
    pub session: Session,
    pub idempotency: Idempotency,
    pub confirmation: Confirmation,
    /// Provenance, not a guardrail: see [`Policy::version`]. Private and
    /// `#[serde(skip)]` so it can only ever be set by [`Policy::load`] (from
    /// the file bytes) or [`Policy::default`] (the sentinel) — never by the
    /// TOML, and never by a caller mutating a loaded policy's numbers.
    #[serde(skip)]
    version: String,
}

/// `Policy`'s equality is *numeric*, not provenance: two policies are equal
/// when every effective guardrail value matches, regardless of which file (or
/// no file) they came from. Hand-written rather than derived precisely so
/// [`Policy::version`] is excluded — a caller comparing policies wants to know
/// "do these enforce the same thing?", and that question must not be answered
/// `false` just because one was loaded from disk and the other built in code.
/// Compare [`Policy::version`] directly when you mean provenance.
impl PartialEq for Policy {
    fn eq(&self, other: &Self) -> bool {
        let Policy {
            kill_switch,
            instruments,
            order_shape,
            price,
            caps,
            margin,
            session,
            idempotency,
            confirmation,
            version: _,
        } = self;
        *kill_switch == other.kill_switch
            && *instruments == other.instruments
            && *order_shape == other.order_shape
            && *price == other.price
            && *caps == other.caps
            && *margin == other.margin
            && *session == other.session
            && *idempotency == other.idempotency
            && *confirmation == other.confirmation
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct KillSwitch {
    pub force_engaged: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Instruments {
    pub exchange: String,
    pub series: Vec<String>,
    pub symbol_allowlist: Vec<String>,
    pub require_in_instrument_dump: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OrderShape {
    pub product: String,
    pub variety: String,
    pub order_type: String,
    pub validity: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub limit_price_band_pct: f64,
    pub max_quote_age_secs: u64,
    pub reject_at_circuit_limit: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Caps {
    pub max_order_value: i64,
    pub max_position_value_per_symbol: i64,
    pub max_orders_per_day: u32,
    pub max_gross_notional_per_day: i64,
    pub max_new_capital_per_day: i64,
    pub min_seconds_between_orders: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Margin {
    pub buffer: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub timezone: String,
    pub open: String,
    pub close: String,
    pub holidays_file: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Idempotency {
    pub window_secs: u64,
    pub price_bucket_pct: f64,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Confirmation {
    pub confirm_all: bool,
    pub confirm_above_value: i64,
    pub confirm_ttl_secs: u64,
}

impl Policy {
    /// The [`Policy::version`] of [`Policy::default`] — the compiled-in
    /// fallback values, which came from no file at all. Deliberately *not*
    /// hash-shaped and not prefixed `sha256:`, so a decision row recorded
    /// under the built-in defaults can never be mistaken for one recorded
    /// under a real operator-owned policy file.
    pub const BUILTIN_VERSION: &'static str = "builtin:defaults";

    /// Prefix of a file-derived [`Policy::version`]. Grep for it to find the
    /// decisions made under a loaded policy file.
    pub const FILE_VERSION_PREFIX: &'static str = "sha256:";

    /// Read and parse a policy file, then run consistency checks.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PolicyError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| PolicyError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let mut policy: Policy = toml::from_str(&text)?;
        policy.validate()?;
        policy.version = content_version(&text);
        Ok(policy)
    }

    /// Opaque identifier for the policy content that was in force.
    ///
    /// `sha256:<16 hex chars>` for a policy read by [`Policy::load`] — the
    /// truncated SHA-256 of the file's bytes with CRLF line endings normalised
    /// to LF, so a Windows and a Unix checkout of the same `policy.toml` agree.
    /// [`Policy::BUILTIN_VERSION`] for [`Policy::default`].
    ///
    /// This is provenance only: it is the hash of the *file*, so it does not
    /// change if a caller mutates a field in memory afterwards (tests do), and
    /// two byte-different files that parse to identical numbers hash
    /// differently. It answers "which file produced this decision?", which is
    /// what DESIGN.md §4 asks of `decisions.policy_version`.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Cheap internal-consistency checks (`tradebot policy check`).
    pub fn validate(&self) -> Result<(), PolicyError> {
        let bad = |m: &str| Err(PolicyError::Inconsistent(m.to_string()));
        if self.price.limit_price_band_pct <= 0.0 {
            return bad("price.limit_price_band_pct must be > 0");
        }
        if self.caps.max_orders_per_day == 0 {
            return bad("caps.max_orders_per_day must be > 0");
        }
        if self.caps.max_order_value > self.caps.max_gross_notional_per_day {
            return bad("caps.max_order_value exceeds caps.max_gross_notional_per_day");
        }
        if self.session.open >= self.session.close {
            return bad("session.open must be earlier than session.close");
        }
        Ok(())
    }

    /// Human-readable dump of the effective values (`tradebot policy check`).
    pub fn effective_summary(&self) -> String {
        format!(
            "policy_version = {}\n\
             kill_switch.force_engaged = {}\n\
             instruments = {} {:?} (allowlist: {})\n\
             order_shape = {}/{}/{}/{}\n\
             price band = \u{00b1}{}%  quote_age <= {}s  reject_at_circuit = {}\n\
             caps: order <= {}  per-symbol <= {}  orders/day <= {}  notional/day <= {}  capital/day <= {}  gap >= {}s\n\
             margin buffer = {}\n\
             session {} {}-{} (holidays: {})\n\
             idempotency: window {}s  price bucket {}%\n\
             confirmation: confirm_all = {}  above {}  ttl {}s",
            self.version,
            self.kill_switch.force_engaged,
            self.instruments.exchange,
            self.instruments.series,
            if self.instruments.symbol_allowlist.is_empty() {
                "any resolvable".to_string()
            } else {
                self.instruments.symbol_allowlist.join(",")
            },
            self.order_shape.product,
            self.order_shape.variety,
            self.order_shape.order_type,
            self.order_shape.validity,
            self.price.limit_price_band_pct,
            self.price.max_quote_age_secs,
            self.price.reject_at_circuit_limit,
            self.caps.max_order_value,
            self.caps.max_position_value_per_symbol,
            self.caps.max_orders_per_day,
            self.caps.max_gross_notional_per_day,
            self.caps.max_new_capital_per_day,
            self.caps.min_seconds_between_orders,
            self.margin.buffer,
            self.session.timezone,
            self.session.open,
            self.session.close,
            self.session.holidays_file,
            self.idempotency.window_secs,
            self.idempotency.price_bucket_pct,
            self.confirmation.confirm_all,
            self.confirmation.confirm_above_value,
            self.confirmation.confirm_ttl_secs,
        )
    }
}

impl Default for Policy {
    fn default() -> Self {
        // Matches the placeholder `policy.toml` at the repo root.
        Policy {
            kill_switch: KillSwitch {
                force_engaged: false,
            },
            instruments: Instruments {
                exchange: "NSE".to_string(),
                series: vec!["EQ".to_string()],
                symbol_allowlist: vec![],
                require_in_instrument_dump: true,
            },
            order_shape: OrderShape {
                product: "CNC".to_string(),
                variety: "regular".to_string(),
                order_type: "LIMIT".to_string(),
                validity: "DAY".to_string(),
            },
            price: Price {
                limit_price_band_pct: 3.0,
                max_quote_age_secs: 15,
                reject_at_circuit_limit: true,
            },
            caps: Caps {
                max_order_value: 25_000,
                max_position_value_per_symbol: 50_000,
                max_orders_per_day: 10,
                max_gross_notional_per_day: 100_000,
                max_new_capital_per_day: 50_000,
                min_seconds_between_orders: 30,
            },
            margin: Margin { buffer: 5_000 },
            session: Session {
                timezone: "Asia/Kolkata".to_string(),
                open: "09:15".to_string(),
                close: "15:30".to_string(),
                holidays_file: "nse_holidays_2026.txt".to_string(),
            },
            idempotency: Idempotency {
                window_secs: 300,
                price_bucket_pct: 0.5,
            },
            confirmation: Confirmation {
                confirm_all: true,
                confirm_above_value: 0,
                confirm_ttl_secs: 120,
            },
            // Built in code, not read from a file — a sentinel, never a hash.
            version: Policy::BUILTIN_VERSION.to_string(),
        }
    }
}

/// Content hash of a policy file's text (DESIGN.md §4).
///
/// CRLF is normalised to LF first: this repo is developed on Windows and
/// `policy.toml` has no `.gitattributes` pin, so the same logical policy can
/// land on disk with either line ending. A version that changed with the
/// checkout's line endings would be worthless for tying a decision to a
/// policy. Truncated to 16 hex chars — 64 bits is far more than enough to
/// distinguish the handful of policy revisions one operator will ever have,
/// and it stays short enough to read in a `decisions` row at a glance.
fn content_version(text: &str) -> String {
    let normalised = text.replace("\r\n", "\n");
    let digest = Sha256::digest(normalised.as_bytes());
    format!(
        "{}{}",
        Policy::FILE_VERSION_PREFIX,
        &hex::encode(digest)[..16]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_self_consistent() {
        Policy::default().validate().unwrap();
    }

    #[test]
    fn repo_policy_toml_parses_and_validates() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../policy.toml");
        let policy = Policy::load(path).expect("policy.toml should load");
        // Numeric equality: `PartialEq` deliberately ignores `version`, so
        // this asserts the repo file enforces exactly the built-in defaults.
        assert_eq!(policy, Policy::default());
        // Provenance is asserted separately, because it is *not* equal: the
        // file-loaded policy carries a real hash, the built-in one a sentinel.
        assert!(
            policy.version().starts_with(Policy::FILE_VERSION_PREFIX),
            "loaded policy.toml should carry a file hash, got {:?}",
            policy.version()
        );
        assert_ne!(policy.version(), Policy::default().version());
    }

    #[test]
    fn default_policy_version_is_the_sentinel_not_a_hash() {
        let default = Policy::default();
        let v = default.version();
        assert_eq!(v, "builtin:defaults");
        assert_eq!(v, Policy::BUILTIN_VERSION);
        assert!(
            !v.starts_with(Policy::FILE_VERSION_PREFIX),
            "the built-in sentinel must not look like a file hash"
        );
    }

    /// Minimal valid policy text, parameterised on one value so a test can
    /// change a single guardrail number and nothing else.
    fn policy_text(max_order_value: i64) -> String {
        format!(
            "\
[kill_switch]
force_engaged = false

[instruments]
exchange = \"NSE\"
series = [\"EQ\"]
symbol_allowlist = []
require_in_instrument_dump = true

[order_shape]
product = \"CNC\"
variety = \"regular\"
order_type = \"LIMIT\"
validity = \"DAY\"

[price]
limit_price_band_pct = 3.0
max_quote_age_secs = 15
reject_at_circuit_limit = true

[caps]
max_order_value = {max_order_value}
max_position_value_per_symbol = 50000
max_orders_per_day = 10
max_gross_notional_per_day = 100000
max_new_capital_per_day = 50000
min_seconds_between_orders = 30

[margin]
buffer = 5000

[session]
timezone = \"Asia/Kolkata\"
open = \"09:15\"
close = \"15:30\"
holidays_file = \"nse_holidays_2026.txt\"

[idempotency]
window_secs = 300
price_bucket_pct = 0.5

[confirmation]
confirm_all = true
confirm_above_value = 0
confirm_ttl_secs = 120
"
        )
    }

    fn load_text(text: &str) -> Policy {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("policy.toml");
        std::fs::write(&path, text).expect("write policy");
        Policy::load(&path).expect("temp policy should load")
    }

    #[test]
    fn version_is_stable_across_loads_of_identical_content() {
        let text = policy_text(25_000);
        // Two separate files in two separate temp dirs: the version must
        // depend on content only, never on path or load time.
        let a = load_text(&text);
        let b = load_text(&text);
        assert_eq!(a.version(), b.version());
        assert!(a.version().starts_with(Policy::FILE_VERSION_PREFIX));
        // `sha256:` + 16 hex chars.
        assert_eq!(a.version().len(), Policy::FILE_VERSION_PREFIX.len() + 16);
    }

    #[test]
    fn changed_guardrail_value_changes_the_version() {
        let a = load_text(&policy_text(25_000));
        let b = load_text(&policy_text(24_000));
        assert_ne!(
            a.version(),
            b.version(),
            "editing max_order_value must produce a different policy version"
        );
    }

    #[test]
    fn crlf_and_lf_checkouts_hash_identically() {
        let lf = policy_text(25_000);
        assert!(!lf.contains('\r'), "fixture should be LF to start with");
        let crlf = lf.replace('\n', "\r\n");
        assert!(crlf.contains("\r\n"));
        assert_eq!(
            load_text(&lf).version(),
            load_text(&crlf).version(),
            "line endings are not policy content"
        );
    }

    #[test]
    fn effective_summary_shows_the_version_to_the_operator() {
        let summary = Policy::default().effective_summary();
        assert!(
            summary.contains(Policy::BUILTIN_VERSION),
            "`tradebot policy check` must print the policy version; got:\n{summary}"
        );
        let loaded = load_text(&policy_text(25_000));
        assert!(loaded.effective_summary().contains(loaded.version()));
    }

    #[test]
    fn unknown_key_is_rejected() {
        let err = toml::from_str::<KillSwitch>("force_engaged = false\nbogus = 1\n").unwrap_err();
        assert!(err.to_string().contains("bogus") || err.to_string().contains("unknown"));
    }

    #[test]
    fn version_cannot_be_injected_through_the_toml() {
        // Top-level, before the first table, so this is a `Policy` key and
        // not a stray key inside the last section.
        let text = format!(
            "version = \"sha256:deadbeefdeadbeef\"\n{}",
            policy_text(25_000)
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("policy.toml");
        std::fs::write(&path, &text).expect("write policy");
        let err = Policy::load(&path).expect_err("`version` is not part of the TOML schema");
        assert!(matches!(err, PolicyError::Parse(_)), "got {err:?}");
    }
}
