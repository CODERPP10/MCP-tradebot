//! Integer money. All amounts are whole paise (1 rupee = 100 paise) so there is
//! no floating-point drift on the order-value and daily-notional checks.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A rupee amount, stored as a signed count of paise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Money {
    paise: i64,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MoneyError {
    #[error("money arithmetic overflowed")]
    Overflow,
    #[error("amount has more precision than one paise: {0}")]
    SubPaise(f64),
}

impl Money {
    pub const ZERO: Money = Money { paise: 0 };

    pub const fn from_paise(paise: i64) -> Self {
        Self { paise }
    }

    pub const fn from_rupees(rupees: i64) -> Self {
        Self {
            paise: rupees * 100,
        }
    }

    /// Convert a rupee figure coming from an external quote (Kite returns
    /// `last_price` as a float). Rejects anything finer than a paise.
    pub fn from_rupees_f64(rupees: f64) -> Result<Self, MoneyError> {
        let paise = rupees * 100.0;
        if !paise.is_finite() {
            return Err(MoneyError::Overflow);
        }
        if (paise.round() - paise).abs() > 1e-6 {
            return Err(MoneyError::SubPaise(rupees));
        }
        Ok(Self {
            paise: paise.round() as i64,
        })
    }

    pub const fn paise(self) -> i64 {
        self.paise
    }

    pub fn rupees_f64(self) -> f64 {
        self.paise as f64 / 100.0
    }

    pub fn checked_add(self, rhs: Money) -> Result<Money, MoneyError> {
        self.paise
            .checked_add(rhs.paise)
            .map(Money::from_paise)
            .ok_or(MoneyError::Overflow)
    }

    /// Order value: unit price times a share count.
    pub fn checked_mul_qty(self, qty: u32) -> Result<Money, MoneyError> {
        self.paise
            .checked_mul(i64::from(qty))
            .map(Money::from_paise)
            .ok_or(MoneyError::Overflow)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.paise < 0 { "-" } else { "" };
        let abs = self.paise.unsigned_abs();
        write!(f, "{sign}\u{20b9}{}.{:02}", abs / 100, abs % 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_rupees_and_paise() {
        assert_eq!(Money::from_paise(150_025).to_string(), "\u{20b9}1500.25");
        assert_eq!(Money::from_paise(-5).to_string(), "-\u{20b9}0.05");
    }

    #[test]
    fn from_rupees_f64_rejects_sub_paise() {
        assert_eq!(Money::from_rupees_f64(10.005), Err(MoneyError::SubPaise(10.005)));
        assert_eq!(Money::from_rupees_f64(10.5).unwrap(), Money::from_paise(1050));
    }

    #[test]
    fn order_value_uses_checked_arithmetic() {
        let px = Money::from_rupees(200);
        assert_eq!(px.checked_mul_qty(3).unwrap(), Money::from_rupees(600));
        assert_eq!(Money::from_paise(i64::MAX).checked_mul_qty(2), Err(MoneyError::Overflow));
    }
}
