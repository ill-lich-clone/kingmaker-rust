use kingmaker_core::{ExternalId, Money};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MoneyMathError {
    #[error("Negative value not allowed")]
    NegativeValue,
    #[error("Invalid or non-finite rate")]
    NonFinite,
    #[error("Invalid denominator")]
    InvalidDenominator,
    #[error("Numeric overflow")]
    Overflow,
    #[error("Parse error")]
    ParseError,
}

pub use kingmaker_core::production::MoneyRate;

#[derive(Clone, Debug)]
pub struct BudgetRequest {
    pub id: ExternalId,
    pub amount: Money,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetAllocation {
    pub id: ExternalId,
    pub paid: Money,
}

pub fn decimal_ratio(text: &str) -> Result<(BigInt, BigInt), MoneyMathError> {
    let s = text.trim();
    let f: f64 = s.parse().map_err(|_| MoneyMathError::ParseError)?;
    if !f.is_finite() {
        return Err(MoneyMathError::NonFinite);
    }

    let (sign, s) = if let Some(rest) = s.strip_prefix('-') {
        (-BigInt::from(1), rest)
    } else if let Some(rest) = s.strip_prefix('+') {
        (BigInt::from(1), rest)
    } else {
        (BigInt::from(1), s)
    };

    let s_lower = s.to_ascii_lowercase();
    let mut exp_parts = s_lower.splitn(2, 'e');
    let base = exp_parts.next().unwrap();
    let exp_str = exp_parts.next().unwrap_or("0");
    let exp: i64 = exp_str.parse().map_err(|_| MoneyMathError::ParseError)?;

    let mut dot_parts = base.splitn(2, '.');
    let integer = dot_parts.next().unwrap();
    let fraction = dot_parts.next().unwrap_or("");

    let power = (fraction.len() as i64)
        .checked_sub(exp)
        .ok_or(MoneyMathError::Overflow)?;

    // Runtime ratios come from finite f64 values (at most 324 decimal places).
    // Reject pathological external text before allocating an unbounded power.
    if power.unsigned_abs() > 10_000 {
        return Err(MoneyMathError::Overflow);
    }
    let digits = format!("{}{}", integer, fraction);
    if digits.is_empty() {
        return Err(MoneyMathError::ParseError);
    }
    let base_num: BigInt = digits.parse().map_err(|_| MoneyMathError::ParseError)?;
    let n = sign * base_num;

    if power >= 0 {
        let p = usize::try_from(power).map_err(|_| MoneyMathError::Overflow)?;
        let d = num_traits::pow(BigInt::from(10), p);
        Ok((n, d))
    } else {
        let p = usize::try_from(-power).map_err(|_| MoneyMathError::Overflow)?;
        let mult = num_traits::pow(BigInt::from(10), p);
        Ok((n * mult, BigInt::from(1)))
    }
}

pub fn round_ratio(n: &BigInt, d: &BigInt) -> Result<BigInt, MoneyMathError> {
    if *d <= BigInt::zero() {
        return Err(MoneyMathError::InvalidDenominator);
    }
    let sign = if *n < BigInt::zero() {
        -BigInt::from(1)
    } else {
        BigInt::from(1)
    };
    let abs_n = n.abs();
    let half_d = d / 2;
    let quotient = (abs_n + half_d) / d;
    Ok(sign * quotient)
}

pub fn multiply_money(money: Money, multiplier: f64) -> Result<Money, MoneyMathError> {
    if money.units() < 0 {
        return Err(MoneyMathError::NegativeValue);
    }
    if !multiplier.is_finite() {
        return Err(MoneyMathError::NonFinite);
    }
    if multiplier < 0.0 {
        return Err(MoneyMathError::NegativeValue);
    }
    let (m_n, m_d) = decimal_ratio(&multiplier.to_string())?;
    let n = BigInt::from(money.units()) * m_n;
    let rounded = round_ratio(&n, &m_d)?;
    let units = rounded.to_i64().ok_or(MoneyMathError::Overflow)?;
    if units < 0 {
        return Err(MoneyMathError::NegativeValue);
    }
    Ok(Money(units))
}

pub fn rate_pay(rate: &MoneyRate, fte: f64, multiplier: f64) -> Result<Money, MoneyMathError> {
    if !fte.is_finite() || !multiplier.is_finite() {
        return Err(MoneyMathError::NonFinite);
    }
    if fte < 0.0 || multiplier < 0.0 {
        return Err(MoneyMathError::NegativeValue);
    }

    let r_num: BigInt = rate
        .numerator
        .trim()
        .parse()
        .map_err(|_| MoneyMathError::ParseError)?;
    let r_den: BigInt = rate
        .denominator
        .trim()
        .parse()
        .map_err(|_| MoneyMathError::ParseError)?;

    if r_num < BigInt::zero() {
        return Err(MoneyMathError::NegativeValue);
    }
    if r_den <= BigInt::zero() {
        return Err(MoneyMathError::InvalidDenominator);
    }

    let (f_n, f_d) = decimal_ratio(&fte.to_string())?;
    let (m_n, m_d) = decimal_ratio(&multiplier.to_string())?;

    let n = r_num * f_n * m_n;
    let d = r_den * f_d * m_d;

    let rounded = round_ratio(&n, &d)?;
    let units = rounded.to_i64().ok_or(MoneyMathError::Overflow)?;
    if units < 0 {
        return Err(MoneyMathError::NegativeValue);
    }
    Ok(Money(units))
}

fn external_id_to_string(id: &ExternalId) -> String {
    match id.json() {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    }
}

pub fn allocate_budget(
    requests: &[BudgetRequest],
    budget: Money,
) -> Result<Vec<BudgetAllocation>, MoneyMathError> {
    allocate_big_budget(
        &requests
            .iter()
            .map(|r| (r.id.clone(), BigInt::from(r.amount.0)))
            .collect::<Vec<_>>(),
        budget,
    )
}
pub fn allocate_big_budget(
    requests: &[(ExternalId, BigInt)],
    budget: Money,
) -> Result<Vec<BudgetAllocation>, MoneyMathError> {
    if budget.0 < 0 || requests.iter().any(|r| r.1 < BigInt::zero()) {
        return Err(MoneyMathError::NegativeValue);
    }
    let mut items = requests.iter().enumerate().collect::<Vec<_>>();
    items.sort_by(|(i, a), (j, b)| {
        crate::ordering::utf16_compare(&external_id_to_string(&a.0), &external_id_to_string(&b.0))
            .then(i.cmp(j))
    });
    let total = items.iter().map(|(_, r)| r.1.clone()).sum::<BigInt>();
    let funds = BigInt::from(budget.0);
    if total <= funds {
        return items
            .into_iter()
            .map(|(_, r)| {
                Ok(BudgetAllocation {
                    id: r.0.clone(),
                    paid: Money(r.1.to_i64().ok_or(MoneyMathError::Overflow)?),
                })
            })
            .collect();
    }
    let mut bases = Vec::new();
    let mut remainders = Vec::new();
    let mut used = BigInt::zero();
    for (_, r) in &items {
        let amount = &r.1 * &funds;
        let base = &amount / &total;
        used += &base;
        bases.push(base.to_i64().ok_or(MoneyMathError::Overflow)?);
        remainders.push(amount % &total);
    }
    let remaining = (funds - used).to_usize().ok_or(MoneyMathError::Overflow)?;
    let mut order = (0..items.len()).collect::<Vec<_>>();
    order.sort_by(|&i, &j| remainders[j].cmp(&remainders[i]).then(i.cmp(&j)));
    let mut bonus = vec![false; items.len()];
    for &i in order.iter().take(remaining) {
        bonus[i] = true;
    }
    Ok(items
        .into_iter()
        .enumerate()
        .map(|(i, (_, r))| BudgetAllocation {
            id: r.0.clone(),
            paid: Money(bases[i] + i64::from(bonus[i])),
        })
        .collect())
}
