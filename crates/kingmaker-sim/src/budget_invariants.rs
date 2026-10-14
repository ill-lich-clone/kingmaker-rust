use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::*};
use kingmaker_core::{operations::*, *};
use std::collections::{HashMap, HashSet};
pub(crate) fn verify_budget_session(
    s: &OperationSession,
    ops: &[EconomicOperation],
) -> OpResult<Vec<(ExternalId, Money)>> {
    let mut expected: HashMap<AccountAddress, i128> = s
        .base.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    let mut issuance = HashMap::<ExternalId, i128>::new();
    let mut seen = HashSet::new();
    for op in ops {
        if !seen.insert(&op.event_id) {
            return Err(error("Duplicate budget operation"));
        }
        let mut delta = HashMap::<ExternalId, i128>::new();
        for d in &op.debits {
            *expected.entry(d.address()).or_default() -= i128::from(d.amount_minor.0);
            *delta.entry(d.currency_id.clone()).or_default() -= i128::from(d.amount_minor.0);
        }
        for c in &op.credits {
            *expected.entry(c.address()).or_default() += i128::from(c.amount_minor.0);
            *delta.entry(c.currency_id.clone()).or_default() += i128::from(c.amount_minor.0);
        }
        let authorized = if op.kind == OperationKind::AdventurerIncome {
            let n: Money = serde_json::from_value(op.meta["issuance_minor"].clone())?;
            let id = id_meta(op, "currency_id")?;
            *issuance.entry(id.clone()).or_default() += i128::from(n.0);
            Some((id, i128::from(n.0)))
        } else {
            None
        };
        if delta.iter().any(|(c, n)| {
            *n != authorized
                .as_ref()
                .filter(|(id, _)| id == c)
                .map(|(_, n)| *n)
                .unwrap_or(0)
        }) {
            return Err(error("Unexplained budget money change"));
        }
        if !op.stock_changes.is_empty() {
            return Err(error("Budget changed goods"));
        }
    }
    let after: HashMap<_, _> = s
        .accounts()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    if expected != after || after.values().any(|n| *n < 0) {
        return Err(error("Budget accounts not explained by operations"));
    }
    let mut result = issuance
        .into_iter()
        .map(|(id, n)| {
            Ok((
                id,
                Money(i64::try_from(n).map_err(|_| error("Issuance summary overflow"))?),
            ))
        })
        .collect::<OpResult<Vec<_>>>()?;
    result.sort_by_key(|(id, _)| id.json().to_string());
    Ok(result)
}
