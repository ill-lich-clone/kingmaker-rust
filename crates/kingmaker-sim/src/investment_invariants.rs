use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult};
use kingmaker_core::{
    audit::{CheckStatus, InvariantReport},
    operations::EconomicOperation,
};
use std::collections::{HashMap, HashSet};
pub(crate) fn verify_investment(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    ops: &[EconomicOperation],
) -> OpResult<InvariantReport> {
    let mut money: HashMap<_, i128> = before.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    if money.len() != before.accounts().len() {
        return Err(error("Duplicate investment account"));
    }
    let mut ids = HashSet::new();
    for op in ops {
        if !ids.insert(&op.event_id) || !op.stock_changes.is_empty() || op.obligation.is_some() {
            return Err(error("Invalid investment changes"));
        }
        let mut balance = HashMap::<_, i128>::new();
        for (legs, sign) in [(&op.debits, -1i128), (&op.credits, 1i128)] {
            for leg in legs {
                if leg.amount_minor.0 < 0 {
                    return Err(error("Negative investment leg"));
                }
                *money.entry(leg.address()).or_default() += sign * i128::from(leg.amount_minor.0);
                *balance.entry(&leg.currency_id).or_default() +=
                    sign * i128::from(leg.amount_minor.0);
            }
        }
        if balance.values().any(|n| *n != 0) {
            return Err(error("Investment issued or destroyed money"));
        }
    }
    let actual: HashMap<_, _> = after.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    if actual != money || actual.len() != after.accounts().len() || actual.values().any(|n| *n < 0) {
        return Err(error("Investment money not explained"));
    }
    if before.stockpiles() != after.stockpiles()
        || before.stock_contributions() != after.stock_contributions()
    {
        return Err(error("Investment changed physical goods or creators"));
    }
    if !crate::pop_invariant_projection::same_people_except_accounts(before.pops(),after.pops()) || before.provinces() != after.provinces() {
        return Err(error("Investment changed population"));
    }
    let assignments: Vec<_> = after.resource_operations().values()
        .flat_map(|e| e.base.assignments.clone())
        .chain(
            after.factories().values()
                .filter(|e| e.status.as_deref() == Some("operating"))
                .flat_map(|e| e.base.assignments.clone()),
        )
        .chain(
            after.artisans().values()
                .flat_map(|e| e.base.assignments.clone()),
        )
        .chain(
            after.construction_projects().values()
                .filter(|p| !crate::labor_roster::INACTIVE_PROJECTS.contains(&p.status.as_str()))
                .flat_map(|p| p.assignments.clone()),
        )
        .collect();
    crate::production_activities::validate_production_assignments(&after.pops(), &assignments)?;
    let mut old = before.mobility_data().clone();
    let mut new = after.mobility_data().clone();
    old.project_investments=kingmaker_core::row_store::RowView::empty();
    new.project_investments=kingmaker_core::row_store::RowView::empty();
    if old != new
        || before.factories() != after.factories()
        || before.artisans() != after.artisans()
        || before.storage_facilities() != after.storage_facilities()
        || before.contracts() != after.contracts()
    {
        return Err(error("Investment changed unrelated property rights"));
    }
    Ok(InvariantReport {
        money: CheckStatus::Passed,
        goods: CheckStatus::Passed,
        people: CheckStatus::Passed,
        labor: CheckStatus::Passed,
        rights: CheckStatus::Passed,
    })
}
