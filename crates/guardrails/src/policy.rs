//! Operator-owned guardrail policy (DESIGN.md §7).
//!
//! Loaded from a TOML file the server process cannot write. The struct mirrors
//! `policy.toml` at the repo root. Every field is required in the file — there
//! are no silent defaults for a live policy — but [`Policy::default`] provides
//! the DESIGN.md placeholder values for tests and `--help` output.

use std::path::Path;

use serde::Deserialize;

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

#[derive(Debug, Clone, Deserialize, PartialEq)]
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
    /// Read and parse a policy file, then run consistency checks.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PolicyError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| PolicyError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let policy: Policy = toml::from_str(&text)?;
        policy.validate()?;
        Ok(policy)
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
            "kill_switch.force_engaged = {}\n\
             instruments = {} {:?} (allowlist: {})\n\
             order_shape = {}/{}/{}/{}\n\
             price band = \u{00b1}{}%  quote_age <= {}s  reject_at_circuit = {}\n\
             caps: order <= {}  per-symbol <= {}  orders/day <= {}  notional/day <= {}  capital/day <= {}  gap >= {}s\n\
             margin buffer = {}\n\
             session {} {}-{} (holidays: {})\n\
             idempotency: window {}s  price bucket {}%\n\
             confirmation: confirm_all = {}  above {}  ttl {}s",
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
        }
    }
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
        assert_eq!(policy, Policy::default());
    }

    #[test]
    fn unknown_key_is_rejected() {
        let err = toml::from_str::<KillSwitch>("force_engaged = false\nbogus = 1\n").unwrap_err();
        assert!(err.to_string().contains("bogus") || err.to_string().contains("unknown"));
    }
}
