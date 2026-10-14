use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::{Add, AddAssign, Sub, SubAssign};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum MoneyError {
    #[error("Money overflow occurred")]
    Overflow,
    #[error("Insufficient funds: current balance {current}, required {required}")]
    InsufficientFunds { current: i64, required: i64 },
}

/// Indivisible integer units of money.
/// Prevents floating point inaccuracies and enforces strict conservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Money(pub i64);

impl Serialize for Money {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for Money {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Integer {
            Text(String),
            Number(i64),
            Tagged(TaggedInteger),
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct TaggedInteger {
            #[serde(rename = "__kingmaker_codec_v2__")]
            kind: String,
            value: String,
        }
        match Integer::deserialize(d)? {
            Integer::Number(n) => Ok(Self(n)),
            Integer::Text(s) => s.parse::<i64>().map(Self).map_err(serde::de::Error::custom),
            Integer::Tagged(t) if t.kind == "bigint" => t
                .value
                .parse::<i64>()
                .map(Self)
                .map_err(serde::de::Error::custom),
            Integer::Tagged(_) => Err(serde::de::Error::custom("Money requires a bigint marker")),
        }
    }
}

impl Money {
    pub const ZERO: Money = Money(0);

    pub const fn from_units(units: i64) -> Self {
        Money(units)
    }

    pub const fn units(&self) -> i64 {
        self.0
    }

    pub fn checked_add(self, rhs: Money) -> Option<Money> {
        self.0.checked_add(rhs.0).map(Money)
    }

    pub fn checked_sub(self, rhs: Money) -> Option<Money> {
        self.0.checked_sub(rhs.0).map(Money)
    }
}

impl Add for Money {
    type Output = Money;
    fn add(self, rhs: Money) -> Money {
        Money(self.0.checked_add(rhs.0).expect("Money addition overflow"))
    }
}

impl AddAssign for Money {
    fn add_assign(&mut self, rhs: Money) {
        self.0 = self.0.checked_add(rhs.0).expect("Money addition overflow");
    }
}

impl Sub for Money {
    type Output = Money;
    fn sub(self, rhs: Money) -> Money {
        Money(
            self.0
                .checked_sub(rhs.0)
                .expect("Money subtraction overflow"),
        )
    }
}

impl SubAssign for Money {
    fn sub_assign(&mut self, rhs: Money) {
        self.0 = self
            .0
            .checked_sub(rhs.0)
            .expect("Money subtraction overflow");
    }
}
