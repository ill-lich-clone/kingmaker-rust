use kingmaker_core::world_read::WorldRead;
use crate::{budget_funding::error, operation_session::OpResult};
use kingmaker_core::{
    audit::{CheckStatus, InvariantReport},
    operations::*,
};
use std::collections::{HashMap, HashSet};
pub(crate) fn verify_construction(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    ops: &[EconomicOperation],
) -> OpResult<InvariantReport> {
    let mut accounts: HashMap<_, i128> = before.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    let mut stocks: HashMap<_, f64> = before.stockpiles().values()
        .map(|s| (s.external_id.clone(), s.quantity))
        .collect();
    let mut ids = HashSet::new();
    if accounts.len() != before.accounts().len() || stocks.len() != before.stockpiles().len() {
        return Err(error("Duplicate construction financial or stock identity"));
    }
    for op in ops {
        if !ids.insert(&op.event_id) || op.obligation.is_some() {
            return Err(error("Invalid construction operation history"));
        }
        let mut currency = HashMap::<_, i128>::new();
        for (legs, sign) in [(&op.debits, -1i128), (&op.credits, 1i128)] {
            for leg in legs {
                if leg.amount_minor.0 < 0 {
                    return Err(error("Negative construction money leg"));
                }
                *accounts.entry(leg.address()).or_default() +=
                    sign * i128::from(leg.amount_minor.0);
                *currency.entry(&leg.currency_id).or_default() +=
                    sign * i128::from(leg.amount_minor.0);
            }
        }
        if currency.values().any(|n| *n != 0) {
            return Err(error("Construction issued or destroyed money"));
        }
        for change in &op.stock_changes {
            if op.kind != OperationKind::ConstructionMaterials
                || change.delta > 0.
                || change.create.is_some()
            {
                return Err(error("Unexplained construction goods"));
            }
            *stocks
                .get_mut(&change.stock_id)
                .ok_or_else(|| error("Construction stock missing"))? += change.delta;
        }
    }
    let actual: HashMap<_, _> = after.accounts().values()
        .map(|a| (a.address(), i128::from(a.balance.0)))
        .collect();
    if actual != accounts || actual.len() != after.accounts().len() || actual.values().any(|n| *n < 0)
    {
        return Err(error("Construction money not explained"));
    }
    if before.stockpiles().len() != after.stockpiles().len() {
        return Err(error("Construction created stock records"));
    }
    for (old, new) in before.stockpiles().values().zip(&after.stockpiles()) {
        let mut expected = old.clone();
        expected.quantity = stocks[&old.external_id];
        if expected != *new || new.quantity < 0. {
            return Err(error("Construction goods or stock rights not explained"));
        }
    }
    if !crate::pop_invariant_projection::same_people_except_accounts(before.pops(),after.pops()) || before.provinces() != after.provinces() {
        return Err(error("Construction changed people"));
    }
    if before.employment() != after.employment()
        || before.contracts() != after.contracts()
        || before.labor_data() != after.labor_data()
    {
        return Err(error("Construction changed labor contracts"));
    }
    if before.mobility_data() != after.mobility_data()
        || before.goods_reservations() != after.goods_reservations()
        || before.storage_reservations() != after.storage_reservations()
    {
        return Err(error("Construction changed unrelated property claims"));
    }
    let mut projects=HashMap::new();
    for project in after.construction_projects().values() {
        if projects.insert(&project.id,project).is_some() {return Err(error("Duplicate construction project identity"));}
    }
    for original in before.construction_projects().values() {
        let p = projects.get(&original.id).copied()
            .ok_or_else(|| error("Construction project lost"))?;
        if p.assignments != original.assignments
            || p.extra.get("owner_id") != original.extra.get("owner_id")
            || p.extra.get("terms") != original.extra.get("terms")
        {
            return Err(error("Construction changed agreed rights or workers"));
        }
        if p.status == "completed" && original.status != "completed" {
            let value = serde_json::to_value(p)?;
            let mut expected =
                crate::construction_rights::complete_project_ownership(after, &value)?["asset"]
                    .clone();
            let mut capital: kingmaker_core::Money =
                serde_json::from_value(value["funded_minor"].clone())?;
            capital.0 -= serde_json::from_value::<kingmaker_core::Money>(
                value
                    .get("refunded_minor")
                    .cloned()
                    .unwrap_or(serde_json::json!("0")),
            )?
            .0;
            if let Some(target) = value.get("target_factory_id").filter(|v| !v.is_null()) {
                let old = before.factories().values()
                    .find(|f| f.base.id.json() == *target)
                    .ok_or_else(|| error("Expanded original factory missing"))?;
                expected = crate::construction_rights::complete_expansion(
                    after,
                    &serde_json::to_value(old)?,
                    &value,
                )?;
                capital = serde_json::from_value(expected["capital_value_minor"].clone())?;
            }
            let asset = if value["kind"] == serde_json::json!("factory") {
                serde_json::to_value(
                    after.factories().values()
                        .find(|f| f.base.id.json() == value["completed_asset_id"])
                        .ok_or_else(|| error("Completed factory missing"))?,
                )?
            } else if matches!(value["kind"].as_str(), Some("mine" | "pasture")) {
                serde_json::to_value(
                    after.expansions().values()
                        .find(|e| e.id == p.id)
                        .ok_or_else(|| error("Completed RGO expansion missing"))?,
                )?["asset"]
                    .clone()
            } else {
                serde_json::to_value(
                    after.completed_construction().values()
                        .find(|e| e.extra.get("id") == Some(&p.id.json()))
                        .ok_or_else(|| error("Completed building missing"))?,
                )?["asset"]
                    .clone()
            };
            if asset["shares"] != expected["shares"]
                || value["shares"] != asset["shares"]
                || serde_json::from_value::<kingmaker_core::Money>(
                    asset["capital_value_minor"].clone(),
                )? != capital
            {
                return Err(error("Completed asset net capital/rights not explained"));
            }
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
