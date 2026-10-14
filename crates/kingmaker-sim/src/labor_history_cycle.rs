//! Closing ages derive exclusively from validated required native phase receipts and committed payroll.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error, labor_history::plan_unemployment_history, operation_session::*,
    profession_cycle::Reads,
};
use kingmaker_core::{
    labor::LaborRequest, market::MarketStage, mobility::LaborHistoryReceipt, operations::*, *,
};
use std::collections::{HashMap, HashSet};
pub struct PreparedLaborHistory {
    reads: Option<Reads>,
    receipt: LaborHistoryReceipt,
}
fn payroll(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<Vec<EconomicOperation>> {
    let budget: Vec<_> = w.all_budget_phase_receipts().select(|p| p.request.period == r.period)?;
    let production: Vec<_> = w.all_production_phase_receipts().select(|p| p.request.period == r.period)?;
    let market: Vec<_> = w.all_market_phase_receipts().select(|p| p.request.period == r.period && p.request.stage == MarketStage::ClosingMarket)?;
    if budget.len() != 1 || production.len() != 1 || market.len() != 1 {
        return Err(error(
            "Labor history requires complete budget, production and closing market",
        ));
    }
    if budget[0].campaign_id != w.campaign().campaign_id
        || production[0].campaign_id != w.campaign().campaign_id
        || market[0].campaign_id != w.campaign().campaign_id
        || budget[0].request.expected_revision != r.expected_revision
        || production[0].request.expected_revision != r.expected_revision
        || market[0].request.expected_revision != r.expected_revision
    {
        return Err(error("Labor history phase campaign/revision mismatch"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut required_payroll = HashSet::new();
    for paid in &production[0].payroll {
        if !required_payroll.insert(&paid.event_id)
            || production[0]
                .event_ids
                .iter()
                .filter(|id| *id == &paid.event_id)
                .count()
                != 1
        {
            return Err(error(
                "Required production payroll event missing or duplicated",
            ));
        }
        let value = history
            .get(&paid.event_id)?
            .ok_or_else(|| error("Required production payroll history missing"))?;
        let op: EconomicOperation = serde_json::from_value((*value).clone())?;
        let committed: kingmaker_core::production::PayrollReceipt =
            serde_json::from_value((*value).clone())?;
        if op.kind != OperationKind::Payroll || committed != *paid {
            return Err(error("Required production payroll history altered"));
        }
    }
    let mut expected = HashSet::new();
    let mut result = vec![];
    for op in budget[0].operations.iter().chain(&market[0].operations) {
        if !history.matches(op) {
            return Err(error("Required phase operation missing or altered"));
        }
        expected.insert(op.event_id.clone());
        if op.kind == OperationKind::Payroll {
            result.push(op.clone());
        }
    }
    for id in &production[0].event_ids {
        let value = history
            .get(id)?
            .ok_or_else(|| error("Required production history missing"))?;
        let op: EconomicOperation = serde_json::from_value((*value).clone())?;
        if op.event_id != *id || !expected.insert(id.clone()) {
            return Err(error("Duplicate/conflicting required phase event"));
        }
        if op.kind == OperationKind::Payroll {
            let paid: kingmaker_core::production::PayrollReceipt =
                serde_json::from_value((*value).clone())?;
            if production[0]
                .payroll
                .iter()
                .filter(|p| p.event_id == *id)
                .count()
                != 1
                || production[0].payroll.iter().find(|p| p.event_id == *id) != Some(&paid)
            {
                return Err(error(
                    "Committed production payroll differs from its required receipt",
                ));
            }
            result.push(op);
        }
    }
    let builders=w.all_construction_phase_receipts().select(|p|p.request.period==r.period && p.stage==kingmaker_core::construction::ConstructionStage::Payroll)?;
    if !builders.is_empty() {
        if builders.len()!=1 {return Err(error("Conflicting builder payroll for labor history"));}
        let request=kingmaker_core::construction::ConstructionRequest{expected_revision:r.expected_revision,period:r.period.clone(),policies:r.policies.clone()};
        if crate::construction_payroll::replay(w,&request)?.as_ref()!=Some(&*builders[0]) {return Err(error("Builder payroll evidence altered"));}
        for op in &builders[0].operations {
            if !expected.insert(op.event_id.clone()) {return Err(error("Duplicate builder labor history"));}
            if op.kind==OperationKind::Payroll {result.push(op.clone());}
        }
    }
    // Older periods are legitimate mandatory history, but cannot reset this period's unpaid ages.
    let mut seen = HashSet::new();
    for op in &result {
        if !seen.insert(&op.event_id)
            || op.meta.get("period").and_then(serde_json::Value::as_str) != Some(&r.period)
        {
            return Err(error("Required payroll period/event mismatch"));
        }
    }
    for op in w.committed_operations().values() {
        if op.value["type"].as_str() == Some("payroll")
            && op.value["period"].as_str() == Some(&r.period)
            && !expected.contains(&op.event_id)
        {
            return Err(error(
                "Orphan payroll without required native phase receipt",
            ));
        }
    }
    Ok(result)
}
fn replay(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<Option<LaborHistoryReceipt>> {
    let rows: Vec<_> = w.all_mobility_history_receipts().select(|p| p.request.period == r.period)?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 || rows[0].request != *r || rows[0].campaign_id != w.campaign().campaign_id {
        return Err(error("Conflicting labor history request"));
    }
    let ops = payroll(w, r)?;
    if ops.iter().map(|op| op.event_id.clone()).collect::<Vec<_>>() != rows[0].payroll_events
        || w.pop_mobility().as_ref() != Some(&rows[0].pop_mobility)
    {
        return Err(error("Labor closing history or payroll evidence altered"));
    }
    Ok(Some(rows[0].clone()))
}
pub fn prepare_labor_history(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<PreparedLaborHistory> {
    if r.expected_revision != w.campaign().revision || r.period.is_empty() {
        return Err(error("Invalid labor history revision/period"));
    }
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedLaborHistory {
            reads: None,
            receipt,
        });
    }
    if w.pop_mobility()
        .as_ref()
        .is_some_and(|m| m["last_completed_period"].as_str() == Some(&r.period))
    {
        return Err(error(
            "Orphan completed labor period without native receipt",
        ));
    }
    let ops = payroll(w, r)?;
    let result = plan_unemployment_history(w, &r.period, &ops)?;
    Ok(PreparedLaborHistory {
        reads: Some(Reads::capture(w)),
        receipt: LaborHistoryReceipt {
            campaign_id: w.campaign().campaign_id.clone(),
            request: r.clone(),
            payroll_events: ops.iter().map(|op| op.event_id.clone()).collect(),
            pop_mobility: result,
        },
    })
}
pub fn apply_labor_history(
    w: &mut WorldState,
    p: PreparedLaborHistory,
) -> OpResult<LaborHistoryReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_labor_history_in(w,p))
}
pub(crate) fn apply_labor_history_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedLaborHistory,
) -> OpResult<LaborHistoryReceipt> {
    if let Some(receipt) = replay(w, &p.receipt.request)? {
        return Ok(receipt);
    }
    if p.reads.as_ref().is_none_or(|r| !r.unchanged(w)) {
        return Err(error("Prepared labor history consulted rows changed"));
    }
    w.set_pop_mobility(Some(p.receipt.pop_mobility.clone()))?;
    let key=w.mobility_history_receipts().next_key().map_err(OperationError)?;
    w.put_mobility_history_receipts(key,p.receipt.clone())?;
    Ok(p.receipt)
}
pub fn execute_labor_history(
    w: &mut WorldState,
    r: &LaborRequest,
) -> OpResult<LaborHistoryReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_labor_history_in(w,r))
}
pub(crate) fn execute_labor_history_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &LaborRequest,
) -> OpResult<LaborHistoryReceipt> {
    let p = prepare_labor_history(w, r)?;
    apply_labor_history_in(w, p)
}
