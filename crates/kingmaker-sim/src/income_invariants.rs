use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult};
use kingmaker_core::{
    audit::{CheckStatus, InvariantReport},
    income::IncomeStage,
    operations::EconomicOperation,
};
use std::collections::{HashMap, HashSet};
pub(crate) fn verify_income(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    ops: &[EconomicOperation],
    stage: IncomeStage,
) -> OpResult<InvariantReport> {
    let mut expected: HashMap<_, i128> = before.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    if expected.len() != before.accounts().len() {
        return Err(error("Duplicate income account address"));
    }
    let mut ids = HashSet::new();
    for op in ops {
        if !ids.insert(&op.event_id) || !op.stock_changes.is_empty() || op.obligation.is_some() {
            return Err(error("Invalid income operation changes"));
        }
        let mut balance: HashMap<_, i128> = HashMap::new();
        for leg in &op.debits {
            if leg.amount_minor.0 < 0 {
                return Err(error("Negative income debit"));
            }
            *expected.entry(leg.address()).or_default() -= i128::from(leg.amount_minor.0);
            *balance.entry(&leg.currency_id).or_default() -= i128::from(leg.amount_minor.0);
        }
        for leg in &op.credits {
            if leg.amount_minor.0 < 0 {
                return Err(error("Negative income credit"));
            }
            *expected.entry(leg.address()).or_default() += i128::from(leg.amount_minor.0);
            *balance.entry(&leg.currency_id).or_default() += i128::from(leg.amount_minor.0);
        }
        if balance.values().any(|n| *n != 0) {
            return Err(error("Income issued or destroyed money"));
        }
    }
    let actual: HashMap<_, i128> = after.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    if expected != actual || actual.len() != after.accounts().len() || actual.values().any(|n| *n < 0)
    {
        return Err(error("Income accounts not explained by payments"));
    }
    if before.stockpiles() != after.stockpiles() {
        return Err(error("Income changed physical goods"));
    }
    if !crate::pop_invariant_projection::same_people_except_accounts(before.pops(),after.pops()) || before.provinces() != after.provinces() {
        return Err(error("Income changed people"));
    }
    if before.employment() != after.employment()
        || before.contracts() != after.contracts()
        || before.labor_data() != after.labor_data()
    {
        return Err(error("Income changed employment or labor"));
    }
    if before.mobility_data() != after.mobility_data()
        || before.storage_facilities() != after.storage_facilities()
        || before.goods_reservations() != after.goods_reservations()
        || before.storage_reservations() != after.storage_reservations()
    {
        return Err(error("Income changed property rights"));
    }
    if before.artisans().len() != after.artisans().len() {
        return Err(error("Income created artisan enterprises"));
    }
    for (old, new) in before.artisans().values()
        .zip(&after.artisans())
    {
        if old.base.id != new.base.id
            || old.base.owner_id != new.base.owner_id
            || old.base.province_id != new.base.province_id
            || old.base.assignments != new.base.assignments
            || old.people_slots != new.people_slots
        {
            return Err(error("Income changed artisan rights or labor"));
        }
        if stage == IncomeStage::ClosingIncome && old.base.method_id != new.base.method_id {
            return Err(error("Closing income activated a pending method early"));
        }
    }
    Ok(InvariantReport {
        money: CheckStatus::Passed,
        goods: CheckStatus::Passed,
        people: CheckStatus::Passed,
        labor: CheckStatus::Passed,
        rights: CheckStatus::Passed,
    })
}
