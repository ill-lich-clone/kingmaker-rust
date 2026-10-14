//! Arrival accounting explains money and goods including cargo; it does not assume fixed supply.
use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult, transport_validation::actor};
use kingmaker_core::{audit::*, operations::*, *};
use std::collections::HashMap;
fn goods(w: &dyn WorldRead) -> OpResult<HashMap<ExternalId, f64>> {
    let mut totals = HashMap::<ExternalId, f64>::new();
    for s in w.stockpiles().values() {
        if !s.quantity.is_finite() || s.quantity < 0. {
            return Err(error("Invalid shipping stock"));
        }
        *totals.entry(s.goods.external_id()).or_default() += s.quantity;
    }
    for c in w.shipments().values() {
        let q = c["remaining_quantity"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.)
            .ok_or_else(|| error("Invalid actual cargo quantity"))?;
        *totals.entry(actor(&c["goods_id"])?).or_default() += q;
    }
    Ok(totals)
}
pub(crate) fn verify_arrivals(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    operations: &[EconomicOperation],
) -> OpResult<InvariantReport> {
    let mut balances = before.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect::<HashMap<_, _>>();
    if balances.len() != before.accounts().len() {
        return Err(error("Duplicate shipping account address"));
    }
    for op in operations {
        let mut currency = HashMap::<&ExternalId, i128>::new();
        for (legs, sign) in [(&op.debits, -1i128), (&op.credits, 1i128)] {
            for l in legs {
                if l.amount_minor.0 < 0 {
                    return Err(error("Negative shipping money leg"));
                }
                *balances.entry(l.address()).or_default() += sign * i128::from(l.amount_minor.0);
                *currency.entry(&l.currency_id).or_default() += sign * i128::from(l.amount_minor.0);
            }
        }
        if currency.values().any(|n| *n != 0) {
            return Err(error("Unexplained shipping money source/sink"));
        }
    }
    let actual = after.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect::<HashMap<_, _>>();
    if actual.len() != after.accounts().len() || actual != balances || actual.values().any(|n| *n < 0)
    {
        return Err(error("Arrival money not explained"));
    }
    let mut debts = before.obligations().values()
        .map(|d| (d.obligation_id.clone(), d.clone()))
        .collect::<HashMap<_, _>>();
    if debts.len() != before.obligations().len() {
        return Err(error("Duplicate shipping obligation"));
    }
    for debt in operations.iter().filter_map(|op| op.obligation.as_ref()) {
        if debt.amount_minor.0 < 0 {
            return Err(error("Negative shipping obligation"));
        }
        debts.insert(debt.obligation_id.clone(), debt.clone());
    }
    let actual_debts = after.obligations().values()
        .map(|d| (d.obligation_id.clone(), d.clone()))
        .collect::<HashMap<_, _>>();
    if actual_debts.len() != after.obligations().len() || actual_debts != debts {
        return Err(error("Shipping obligations not explained"));
    }
    let old = goods(before)?;
    let new = goods(after)?;
    for id in old.keys().chain(new.keys()) {
        let a = old.get(id).copied().unwrap_or(0.);
        let b = new.get(id).copied().unwrap_or(0.);
        if !a.is_finite() || !b.is_finite() || (a - b).abs() > 1e-9 * a.abs().max(1.) {
            return Err(error("Arrival stock/cargo goods not conserved"));
        }
    }
    if !crate::pop_invariant_projection::same_people_except_accounts(before.pops(),after.pops())
        || before.provinces() != after.provinces()
        || before.campaign() != after.campaign()
    {
        return Err(error("Arrivals changed people/date"));
    }
    if before.contracts() != after.contracts()
        || before.employment() != after.employment()
        || before.labor_data() != after.labor_data()
    {
        return Err(error("Arrivals changed labor"));
    }
    if before.factories() != after.factories()
        || before.resource_operations() != after.resource_operations()
        || before.artisans() != after.artisans()
        || before.storage_facilities() != after.storage_facilities()
        || before.goods_reservations() != after.goods_reservations()
        || before.project_investments() != after.project_investments()
    {
        return Err(error("Arrival changed unrelated rights"));
    }
    kingmaker_core::diagnostics::measure("audit.arrivals.stock_rights", kingmaker_core::diagnostics::Visibility::Detail, || -> OpResult<()> {
        // Index once even for slice-backed WorldRead. Keep the per-stock sum in
        // operation order, starting at zero, before adding the opening quantity.
        let mut changes = HashMap::<&ExternalId, f64>::new();
        let mut change_visits=0;
        for change in operations.iter().flat_map(|op| &op.stock_changes) {
            change_visits+=1;
            *changes.entry(&change.stock_id).or_default() += change.delta;
        }
        kingmaker_core::diagnostics::add(kingmaker_core::diagnostics::Metric::AuditStockChanges,change_visits);
        let mut stocks = HashMap::new();
        for stock in after.stockpiles().values() {
            if stocks.insert(&stock.external_id, stock).is_some() {
                return Err(error("Duplicate shipping stock identity"));
            }
        }
        for stock in before.stockpiles().values() {
            let mut expected = stock.clone();
            expected.quantity += changes.get(&stock.external_id).copied().unwrap_or(0.);
            if stocks.get(&stock.external_id).copied() != Some(&expected) {
                return Err(error("Arrival changed old stock rights"));
            }
        }
        Ok(())
    })?;
    Ok(InvariantReport {
        money: CheckStatus::Passed,
        goods: CheckStatus::Passed,
        people: CheckStatus::Passed,
        labor: CheckStatus::Passed,
        rights: CheckStatus::Passed,
    })
}
