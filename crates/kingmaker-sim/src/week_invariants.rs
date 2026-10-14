//! Whole-week actual changes must equal the canonical operations of this transaction.
use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult};
use kingmaker_core::{audit::*, operations::*, *};
use std::collections::{HashMap, HashSet};
fn close(a: f64, b: f64) -> bool {
    a.is_finite() && b.is_finite() && (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.)
}
pub(crate) fn verify_week(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    ops: &[EconomicOperation],
) -> OpResult<InvariantReport> {
    let mut ids = HashSet::new();
    let mut money = before.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect::<HashMap<_, _>>();
    if money.len() != before.accounts().len() {
        return Err(error("Duplicate opening week accounts"));
    }
    let mut stocks = before.stockpiles().values()
        .map(|s| (s.external_id.clone(), s.quantity))
        .collect::<HashMap<_, _>>();
    if stocks.len() != before.stockpiles().len() {
        return Err(error("Duplicate opening stock identity"));
    }
    let titles = before.stockpiles().values()
        .map(|s| (&s.external_id, s))
        .collect::<HashMap<_, _>>();
    for s in after.stockpiles().values() {
        if let Some(old) = titles.get(&s.external_id) {
            if old.goods != s.goods || old.unit != s.unit {
                return Err(error(
                    "Weekly existing stock goods/unit changed without conversion",
                ));
            }
        }
    }
    let mut cargo = HashMap::<ExternalId, f64>::new();
    for s in before.shipments().values() {
        let id = ExternalId::from_json(s["id"].clone()).map_err(|m| error(&m))?;
        if cargo
            .insert(
                id,
                s["remaining_quantity"]
                    .as_f64()
                    .ok_or_else(|| error("Invalid opening cargo"))?,
            )
            .is_some()
        {
            return Err(error("Duplicate opening cargo"));
        }
    }
    for op in ops {
        if !ids.insert(&op.event_id) {
            return Err(error("Duplicate weekly operation"));
        }
        let mut delta = HashMap::<ExternalId, i128>::new();
        for d in &op.debits {
            *money.entry(d.address()).or_default() -= i128::from(d.amount_minor.0);
            *delta.entry(d.currency_id.clone()).or_default() -= i128::from(d.amount_minor.0);
        }
        for c in &op.credits {
            *money.entry(c.address()).or_default() += i128::from(c.amount_minor.0);
            *delta.entry(c.currency_id.clone()).or_default() += i128::from(c.amount_minor.0);
        }
        if delta.values().any(|v| *v != 0)
            && !matches!(
                op.kind,
                OperationKind::AdventurerIncome | OperationKind::PopCohortClaim
            )
        {
            return Err(error("Unexplained weekly money creation"));
        }
        for d in &op.stock_changes {
            *stocks.entry(d.stock_id.clone()).or_default() += d.delta;
        }
        if matches!(
            op.kind,
            OperationKind::ShipmentDispatch
                | OperationKind::ShipmentArrival
                | OperationKind::ShipmentReturnArrival
        ) {
            let id = ExternalId::from_json(
                op.meta
                    .get("shipment_id")
                    .ok_or_else(|| error("Weekly shipment identity absent"))?
                    .clone(),
            )
            .map_err(|m| error(&m))?;
            let q = if op.kind == OperationKind::ShipmentDispatch {
                -op.stock_changes.iter().map(|s| s.delta).sum::<f64>()
            } else {
                op.meta
                    .get("quantity")
                    .and_then(serde_json::Value::as_f64)
                    .ok_or_else(|| error("Weekly arrival quantity absent"))?
            };
            if !q.is_finite() || q < 0. {
                return Err(error("Invalid weekly shipment operation quantity"));
            }
            *cargo.entry(id).or_default() += if op.kind == OperationKind::ShipmentDispatch {
                q
            } else {
                -q
            };
        }
    }
    let actual = after.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect::<HashMap<_, _>>();
    if actual.len() != after.accounts().len() || actual != money || actual.values().any(|n| *n < 0) {
        return Err(error("Weekly account changes not explained"));
    }
    let mut seen = HashSet::new();
    for s in after.stockpiles().values() {
        if !seen.insert(&s.external_id)
            || s.quantity < 0.
            || !close(s.quantity, stocks.remove(&s.external_id).unwrap_or(0.))
        {
            return Err(error("Weekly stock changes not explained"));
        }
    }
    if stocks.values().any(|q| !close(*q, 0.)) {
        return Err(error("Weekly stock reference lost"));
    }
    let mut seen = HashSet::new();
    for s in after.shipments().values() {
        let id = ExternalId::from_json(s["id"].clone()).map_err(|m| error(&m))?;
        let q = s["remaining_quantity"]
            .as_f64()
            .ok_or_else(|| error("Invalid closing cargo"))?;
        if !seen.insert(id.clone()) || q < 0. || !close(q, cargo.remove(&id).unwrap_or(0.)) {
            return Err(error("Weekly cargo changes not explained"));
        }
    }
    if cargo.values().any(|q| !close(*q, 0.)) {
        return Err(error("Weekly cargo reference lost"));
    }
    let people = |w: &dyn WorldRead| w.pops().values().map(|p| u64::from(p.size)).sum::<u64>();
    let workforce = |w: &dyn WorldRead| w.pops().values().map(|p| p.workforce).sum::<f64>();
    if people(before) != people(after) || !close(workforce(before), workforce(after)) {
        return Err(error("Weekly people/workforce not explained"));
    }
    crate::labor_roster::build_labor_roster(after)?;
    Ok(InvariantReport {
        money: CheckStatus::Passed,
        goods: CheckStatus::Passed,
        people: CheckStatus::Passed,
        labor: CheckStatus::Passed,
        rights: CheckStatus::NotMeasured {
            reason:
                "Operational phase titles checked; complete authored property graph not measured"
                    .into(),
        },
    })
}
