//! Actual required native operations justify income; initial stock is separate from output.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    income_basis::{validate_income_basis, IncomeOpeningBasis},
    market_operations::invoice,
    money_math::multiply_money,
    needs::js_string,
    operation_history::validate_production_history,
    operation_session::{canonical_operation, OpResult},
    transport_validation::actor,
};
use kingmaker_core::{
    income::IncomeRequest,
    market::MarketStage,
    operations::{EconomicOperation, OperationKind},
    ExternalId, Money,
};
use num_traits::ToPrimitive;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RecognizedProducerIncome {
    pub owner_id: ExternalId,
    pub currency_id: ExternalId,
    pub revenue_minor: Money,
    pub opening_stock_revenue_minor: Money,
    pub recognized_cost_minor: Option<Money>,
    pub diagnostics: Vec<Value>,
}
pub(crate) fn required_income_operations(
    w: &dyn WorldRead,
    r: &IncomeRequest,
) -> OpResult<Vec<EconomicOperation>> {
    let budget = w.all_budget_phase_receipts().select(|p| p.request.period == r.period)?;
    let production = w.all_production_phase_receipts().select(|p| p.request.period == r.period)?;
    let markets = w.all_market_phase_receipts().select(|p| p.request.period == r.period)?;
    if budget.len() != 1
        || production.len() != 1
        || markets
            .iter()
            .filter(|p| p.request.stage == MarketStage::ClosingMarket)
            .count()
            != 1
    {
        return Err(error(
            "Income requires complete budget, production and closing market",
        ));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut expected = HashMap::new();
    let mut include = |op: EconomicOperation| -> OpResult<()> {
        if !history.matches(&op)
            || expected.insert(op.event_id.clone(), op).is_some()
        {
            return Err(error(
                "Missing/altered/duplicated required income operation",
            ));
        }
        Ok(())
    };
    let mobility = w.all_mobility_opening_receipts().select(|p| p.request.period == r.period)?;
    if mobility.len() > 1 {
        return Err(error("Conflicting income mobility opening"));
    }
    if let Some(p) = mobility.first() {
        if p.request.expected_revision != r.expected_revision
            || crate::mobility_cycle::replay(w, &p.request)?.as_ref() != Some(&**p)
        {
            return Err(error("Income mobility evidence altered"));
        }
        for op in &p.operations {
            include(op.clone())?;
        }
    }
    let opening = w.all_income_phase_receipts().select(|p| {
            p.request.period == r.period
                && p.stage == kingmaker_core::income::IncomeStage::ArtisanOpening
        })?;
    if opening.len() > 1 {
        return Err(error("Conflicting income artisan opening"));
    }
    if let Some(p) = opening.first() {
        if p.request.expected_revision != r.expected_revision
            || crate::income_cycle::replay(w, &p.request, p.stage)?.as_ref() != Some(&**p)
        {
            return Err(error("Income artisan opening evidence altered"));
        }
        for op in &p.operations {
            include(op.clone())?;
        }
    }
    if budget[0].campaign_id != w.campaign().campaign_id
        || budget[0].request.expected_revision != r.expected_revision
        || production[0].request.expected_revision != r.expected_revision
    {
        return Err(error("Income phase campaign/revision mismatch"));
    }
    crate::budget_cycle::validate_budget_receipt(w, &budget[0])?;
    for op in &budget[0].operations {
        include(op.clone())?
    }
    for op in validate_production_history(w, &production[0])? {
        include(op)?
    }
    let construction = w.all_construction_phase_receipts().select(|p| {
            p.request.period == r.period
                && p.stage != kingmaker_core::construction::ConstructionStage::Cancellation
        })?;
    if !construction.is_empty() {
        use kingmaker_core::construction::ConstructionStage;
        if construction.len() != 3 {
            return Err(error(
                "Income requires complete investment, builder payroll and construction history",
            ));
        }
        let mut stages = HashSet::new();
        for p in construction {
            if p.request.expected_revision != r.expected_revision || !stages.insert(p.stage as u8) {
                return Err(error("Conflicting income construction phase"));
            }
            let checked = match p.stage {
                ConstructionStage::Investments => crate::investments::replay(w, &p.request)?,
                ConstructionStage::Payroll => crate::construction_payroll::replay(w, &p.request)?,
                ConstructionStage::Progress => crate::construction_cycle::replay(w, &p.request)?,
                ConstructionStage::Cancellation => unreachable!(),
            };
            if checked.as_ref() != Some(&*p) {
                return Err(error("Income construction evidence altered"));
            }
            for op in &p.operations {
                include(op.clone())?;
            }
        }
    }
    let mut stages = HashSet::new();
    for phase in markets {
        if !stages.insert(std::mem::discriminant(&phase.request.stage))
            || phase.campaign_id != w.campaign().campaign_id
            || phase.request.expected_revision != r.expected_revision
        {
            return Err(error("Conflicting income market phase"));
        }
        for op in &phase.operations {
            include(op.clone())?
        }
    }
    for stage in [
        kingmaker_core::transport::TransportStage::Arrivals,
        kingmaker_core::transport::TransportStage::LocalFoodReserves,
        kingmaker_core::transport::TransportStage::Dispatch,
        kingmaker_core::transport::TransportStage::StateProcurement,
    ] {
        let rows = w.all_transport_phase_receipts().select(|p| p.request.period == r.period && p.request.stage == stage)?;
        if rows.len() != 1 {
            return Err(error("Income requires arrivals, local food reserves, physical supply and raw procurement"));
        }
        let p = &*rows[0];
        let request = kingmaker_core::transport::TransportRequest {
            expected_revision: r.expected_revision,
            period: r.period.clone(),
            stage,
        };
        let checked = match stage {
            kingmaker_core::transport::TransportStage::Arrivals => {
                crate::shipment_arrivals::replay(w, &request)?
            }
            kingmaker_core::transport::TransportStage::LocalFoodReserves => {
                crate::local_food_reserves::replay(w, &request)?
            }
            kingmaker_core::transport::TransportStage::Dispatch => {
                crate::physical_supply::replay(w, &request)?
            }
            kingmaker_core::transport::TransportStage::StateProcurement => {
                let policies = p
                    .operations
                    .last()
                    .and_then(|o| o.meta.get("policies"))
                    .ok_or_else(|| error("Procurement policy history missing"))?;
                crate::raw_procurement::replay(
                    w,
                    &kingmaker_core::budget::BudgetRequest {
                        expected_revision: r.expected_revision,
                        period: r.period.clone(),
                        policies: serde_json::from_value(policies.clone())?,
                    },
                )?
            }
            _ => unreachable!(),
        };
        if checked.as_ref() != Some(p) {
            return Err(error("Income shipping evidence altered"));
        }
        for op in &p.operations {
            include(op.clone())?;
        }
    }
    // Actor events use their event ID for replay, and the campaign date/revision for economic attribution.
    for p in w.transport_phase_receipts().values().filter(|p| {
        matches!(
            p.request.stage,
            kingmaker_core::transport::TransportStage::Interruption
                | kingmaker_core::transport::TransportStage::Recovery
                | kingmaker_core::transport::TransportStage::Return
        ) && p.request.expected_revision == r.expected_revision
    }) {
        let action = p
            .decisions
            .first()
            .ok_or_else(|| error("Shipment action request lost"))?;
        let checked = crate::shipment_actions::replay(w, &serde_json::from_value(action.clone())?)?;
        if checked.as_ref() != Some(p)
            || p.date != kingmaker_core::calendar::GameDate::from(w.campaign())
        {
            return Err(error("Income shipment action evidence altered"));
        }
        for op in &p.operations {
            include(op.clone())?;
        }
    }
    let mut result = vec![];
    for op in w.committed_operations().values() {
        if op.value["type"].as_str() == Some("supply_phase_boundary")
            && op.value["economic_revision"].as_u64() == Some(r.expected_revision)
            && !expected.contains_key(&op.event_id)
        {
            return Err(error("Orphan shipment action without income evidence"));
        }
        if op.value["period"].as_str() == Some(&r.period)
            && matches!(
                op.value["type"].as_str(),
                Some(
                    "purchase"
                        | "production"
                        | "payroll"
                        | "storage_rent"
                        | "shipment_arrival"
                        | "shipment_dispatch"
                        | "shipment_recovery"
                        | "shipment_return_dispatch"
                        | "shipment_return_arrival"
                        | "supply_phase_boundary"
                        | "state_procurement_period"
                )
            )
            && !expected.contains_key(&op.event_id)
        {
            return Err(error(
                "Orphan economic operation without required income phase evidence",
            ));
        }
        if let Some(value) = expected.remove(&op.event_id) {
            result.push(value)
        }
    }
    if !expected.is_empty() {
        return Err(error("Required income operation lost"));
    }
    Ok(result)
}
fn plus(a: Money, b: Money) -> OpResult<Money> {
    a.0.checked_add(b.0)
        .map(Money)
        .ok_or_else(|| error("Recognized income overflow"))
}
fn row(
    rows: &mut Vec<RecognizedProducerIncome>,
    indices: &mut HashMap<(ExternalId, ExternalId), usize>,
    owner: &ExternalId,
    currency: &ExternalId,
) -> usize {
    let key = (owner.clone(), currency.clone());
    if let Some(index) = indices.get(&key) {
        return *index;
    }
    let index = rows.len();
    rows.push(RecognizedProducerIncome {
        owner_id: owner.clone(),
        currency_id: currency.clone(),
        revenue_minor: Money::ZERO,
        opening_stock_revenue_minor: Money::ZERO,
        recognized_cost_minor: Some(Money::ZERO),
        diagnostics: vec![],
    });
    indices.insert(key, index);
    index
}
fn cost(row: &mut RecognizedProducerIncome, amount: Money) -> OpResult<()> {
    if let Some(old) = row.recognized_cost_minor {
        row.recognized_cost_minor = Some(plus(old, amount)?)
    }
    Ok(())
}
pub fn recognize_producer_income(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    basis: &IncomeOpeningBasis,
) -> OpResult<Vec<RecognizedProducerIncome>> {
    validate_income_basis(w, r, basis)?;
    let operations = required_income_operations(w, r)?;
    recognize_producer_operations(w, basis, &operations)
}
pub(crate) fn recognize_producer_operations(
    w: &dyn WorldRead,
    basis: &IncomeOpeningBasis,
    operations: &[EconomicOperation],
) -> OpResult<Vec<RecognizedProducerIncome>> {
    let subjects = w.resource_operations().values()
        .map(|p| &p.base)
        .chain(w.factories().values().map(|p| &p.base))
        .map(|p| (&p.id, p))
        .collect::<HashMap<_, _>>();
    let owners = subjects
        .values()
        .filter_map(|p| p.owner_id.as_ref())
        .collect::<HashSet<_>>();
    let stocks = basis
        .stocks
        .iter()
        .chain(&w.stockpiles())
        .map(|s| (&s.external_id, s))
        .collect::<HashMap<_, _>>();
    let mut rows = vec![];
    let mut indices = HashMap::new();
    for op in operations {
        match op.kind {
            OperationKind::ShipmentArrival => {
                for credit in &op.credits {
                    if owners.contains(&credit.owner_id) {
                        let index = row(
                            &mut rows,
                            &mut indices,
                            &credit.owner_id,
                            &credit.currency_id,
                        );
                        if op.meta.get("goods_origin").and_then(Value::as_str) == Some("produced") {
                            rows[index].revenue_minor =
                                plus(rows[index].revenue_minor, credit.amount_minor)?
                        } else {
                            rows[index].opening_stock_revenue_minor =
                                plus(rows[index].opening_stock_revenue_minor, credit.amount_minor)?
                        }
                    }
                }
            }
            OperationKind::ShipmentDispatch => {
                let id = actor(&op.meta["shipment_id"])?;
                let shipment = w.shipments().values()
                    .find(|s| s.get("id") == Some(&id.json()))
                    .ok_or_else(|| error("Income shipment reference missing"))?;
                let owner = actor(&shipment["buyer_id"])?;
                if owners.contains(&owner) {
                    let currency = actor(&shipment["currency_id"])?;
                    let index = row(&mut rows, &mut indices, &owner, &currency);
                    cost(
                        &mut rows[index],
                        serde_json::from_value(shipment["freight_paid_minor"].clone())?,
                    )?
                }
            }
            OperationKind::ShipmentRecovery | OperationKind::ShipmentReturnDispatch => {
                for debit in &op.debits {
                    if owners.contains(&debit.owner_id) {
                        let index =
                            row(&mut rows, &mut indices, &debit.owner_id, &debit.currency_id);
                        cost(&mut rows[index], debit.amount_minor)?
                    }
                }
            }
            OperationKind::Purchase => {
                for credit in &op.credits {
                    if !owners.contains(&credit.owner_id) {
                        continue;
                    }
                    let sold = op
                        .stock_changes
                        .iter()
                        .filter(|c| {
                            c.delta < 0.
                                && stocks
                                    .get(&c.stock_id)
                                    .is_some_and(|s| s.owner_id == credit.owner_id)
                        })
                        .collect::<Vec<_>>();
                    let total = sold.iter().map(|c| -c.delta).sum::<f64>();
                    let produced = sold
                        .iter()
                        .filter(|c| {
                            stocks[&c.stock_id].inventory_origin.as_deref() == Some("produced")
                        })
                        .map(|c| -c.delta)
                        .sum::<f64>();
                    let earned = if total > 0. {
                        multiply_money(credit.amount_minor, produced / total)?
                    } else {
                        Money::ZERO
                    };
                    let index = row(
                        &mut rows,
                        &mut indices,
                        &credit.owner_id,
                        &credit.currency_id,
                    );
                    rows[index].revenue_minor = plus(rows[index].revenue_minor, earned)?;
                    rows[index].opening_stock_revenue_minor = plus(
                        rows[index].opening_stock_revenue_minor,
                        Money(credit.amount_minor.0 - earned.0),
                    )?;
                }
                if op
                    .meta
                    .get("maintenance")
                    .is_some_and(crate::needs::js_truthy)
                {
                    for debit in &op.debits {
                        if owners.contains(&debit.owner_id) {
                            let index =
                                row(&mut rows, &mut indices, &debit.owner_id, &debit.currency_id);
                            cost(&mut rows[index], debit.amount_minor)?
                        }
                    }
                }
            }
            OperationKind::StorageRent => {
                for debit in &op.debits {
                    if owners.contains(&debit.owner_id) {
                        let index =
                            row(&mut rows, &mut indices, &debit.owner_id, &debit.currency_id);
                        cost(&mut rows[index], debit.amount_minor)?
                    }
                }
            }
            OperationKind::Payroll => {
                let owner = actor(&op.meta["employer_id"])?;
                if owners.contains(&owner) {
                    let currency = actor(&op.meta["currency_id"])?;
                    let index = row(&mut rows, &mut indices, &owner, &currency);
                    cost(
                        &mut rows[index],
                        serde_json::from_value(op.meta["paid_gross_minor"].clone())?,
                    )?
                }
            }
            OperationKind::Production => {
                let subject = actor(&op.meta["subject_id"])?;
                let Some(e) = subjects.get(&subject) else {
                    continue;
                };
                let owner = e
                    .owner_id
                    .as_ref()
                    .ok_or_else(|| error("Recognized producer owner missing"))?;
                for change in op.stock_changes.iter().filter(|c| c.delta < 0.) {
                    let stock = stocks
                        .get(&change.stock_id)
                        .ok_or_else(|| error("Recognized production input stock missing"))?;
                    let currency = stock
                        .currency_id
                        .as_ref()
                        .or_else(|| {
                            w.accounts().values()
                                .find(|a| {
                                    &a.owner_id == owner
                                        && a.purpose == "employer_operating_capital"
                                })
                                .map(|a| &a.currency_id)
                        })
                        .ok_or_else(|| error("Input cost currency unresolved"))?;
                    let price = w.price_calibration()
                        .as_ref()
                        .and_then(|p| p.prices_by_currency.get(&js_string(&currency.json())))
                        .and_then(|p| p.get(&js_string(&stock.goods.external_id().json())))
                        .and_then(Value::as_f64)
                        .filter(|p| p.is_finite() && *p > 0.);
                    let index = row(&mut rows, &mut indices, owner, currency);
                    if let Some(price) = price {
                        let amount = invoice(-change.delta, price)?
                            .to_i64()
                            .map(Money)
                            .ok_or_else(|| error("Recognized input invoice overflow"))?;
                        cost(&mut rows[index], amount)?
                    } else {
                        rows[index].recognized_cost_minor = None;
                        rows[index].diagnostics.push(json!({"id":subject,"reason":"producer_input_cost_basis_unresolved","stock_id":stock.external_id,"currency_id":currency,"measurement":"нет измерения"}));
                    }
                }
            }
            _ => {}
        }
    }
    Ok(rows)
}
