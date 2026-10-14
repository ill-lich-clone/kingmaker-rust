use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::*, budget_invariants::verify_budget_session, operation_session::*,
    production_payroll::plan_prior_production_obligations, service_payroll::*,
};
use kingmaker_core::operations::{EconomicOperation, OperationKind};
use kingmaker_core::{budget::*, *};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
struct BudgetReadSet { hashes: Option<Vec<[u8;32]>> }
impl BudgetReadSet {
    fn capture(w:&dyn WorldRead)->Self {
        let hashes=(|| -> OpResult<Vec<[u8;32]>> { Ok(vec![
            crate::read_guard::fingerprint(&w.pops())?,
            crate::read_guard::fingerprint(&w.accounts())?,
            crate::read_guard::fingerprint(&w.resource_operations())?,
            crate::read_guard::fingerprint(&w.factories())?,
            crate::read_guard::fingerprint(&w.artisans())?,
            crate::read_guard::fingerprint(&w.contracts())?,
            crate::read_guard::fingerprint(&w.expansions())?,
            crate::read_guard::fingerprint(&w.construction_projects())?,
            crate::read_guard::fingerprint(&w.completed_construction())?,
            crate::read_guard::fingerprint(&w.storage_facilities())?,
            crate::read_guard::fingerprint(&w.storage_reservations())?,
            crate::read_guard::fingerprint(&w.goods_reservations())?,
            crate::read_guard::fingerprint(&w.money_reservations())?,
            crate::read_guard::fingerprint(&w.obligations())?,
            crate::read_guard::fingerprint(&w.all_committed_operations())?,
            crate::read_guard::fingerprint(&w.maintenance_agreements())?,
            crate::read_guard::fingerprint(&w.employment())?,
            crate::read_guard::fingerprint(&w.adventurer_income_rates())?,
            crate::read_guard::fingerprint(&w.institutions())?,
            crate::read_guard::fingerprint(&w.jobs())?,
            crate::read_guard::fingerprint(&w.all_production_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_budget_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_budget_opening_receipts())?,
            crate::read_guard::fingerprint(&w.campaign())?,
            crate::read_guard::fingerprint(&w.production_policies())?,
            crate::read_guard::fingerprint(&w.production_catalog())?,
            crate::read_guard::fingerprint(&w.budget_profile_id())?,
            crate::read_guard::fingerprint(&w.foundation_budget_policy())?,
        ]) })().ok();
        Self {hashes}
    }
    fn unchanged(&self,w:&dyn WorldRead)->bool { self.hashes.is_some() && self.hashes==Self::capture(w).hashes }
}
pub struct PreparedBudget {
    changes: Option<PreparedChanges>,
    reads: Option<BudgetReadSet>,
    receipt: BudgetPhaseReceipt,
    opening: bool,
    opening_receipt: Option<BudgetPhaseReceipt>,
}
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Opening,
    Complete,
    Services,
}
fn proof(p: &BudgetPhaseReceipt, operations: &[EconomicOperation]) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(&json!({"campaign_id":p.campaign_id,"request":p.request,"operations":operations,"issuance":p.issuance_by_currency,"requirements":p.requirements}))?).iter().map(|b|format!("{b:02x}")).collect())
}
fn append_boundary(
    s: &mut OperationSession,
    p: &mut BudgetPhaseReceipt,
    stage: &str,
) -> OpResult<()> {
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":format!("budget-phase:{}:{}:{stage}",p.campaign_id,p.request.period),"type":"budget_phase_boundary","period":p.request.period,"stage":stage,"effective_policies":s.policies,"receipt_sha256":proof(p,&p.operations)?}),
    )?;
    s.stage_budget(&op)?;
    p.operations.push(op);
    Ok(())
}
pub(crate) fn validate_budget_receipt(w: &dyn WorldRead, p: &BudgetPhaseReceipt) -> OpResult<()> {
    let marker = p
        .operations
        .last()
        .ok_or_else(|| error("Budget boundary lost"))?;
    let stage = marker
        .meta
        .get("stage")
        .and_then(Value::as_str)
        .filter(|v| matches!(*v, "opening" | "services"))
        .ok_or_else(|| error("Budget boundary stage altered"))?;
    if marker.kind != OperationKind::BudgetPhaseBoundary
        || marker.event_id.json()
            != json!(format!(
                "budget-phase:{}:{}:{stage}",
                p.campaign_id, p.request.period
            ))
        || marker.meta.get("receipt_sha256")
            != Some(&json!(proof(p, &p.operations[..p.operations.len() - 1])?))
    {
        return Err(error("Budget receipt evidence altered"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    if p.operations
            .iter()
            .any(|op| !history.matches(op))
    {
        return Err(error("Missing/conflicting budget history"));
    }
    Ok(())
}
fn replay(
    w: &dyn WorldRead,
    r: &BudgetRequest,
    opening: bool,
) -> OpResult<Option<BudgetPhaseReceipt>> {
    let matching = (if opening {
        w.all_budget_opening_receipts()
    } else {
        w.all_budget_phase_receipts()
    })
    .select(|p| p.request.period == r.period)?;
    if matching.is_empty() {
        return Ok(None);
    }
    if matching.len() != 1 {
        return Err(error("Duplicate budget receipt"));
    }
    let p = &*matching[0];
    if p.request != *r || p.campaign_id != w.campaign().campaign_id {
        return Err(error("Conflicting budget request"));
    }
    validate_budget_receipt(w, p)?;
    if p.operations.last().unwrap().meta["stage"]
        != json!(if opening { "opening" } else { "services" })
    {
        return Err(error("Budget phase stage altered"));
    }
    Ok(Some(p.clone()))
}
pub fn prepare_budget_cycle(w: &dyn WorldRead, r: &BudgetRequest) -> OpResult<PreparedBudget> {
    prepare(w, r, Mode::Complete)
}
fn prepare(w: &dyn WorldRead, r: &BudgetRequest, mode: Mode) -> OpResult<PreparedBudget> {
    if r.period.is_empty() || w.campaign().revision != r.expected_revision {
        return Err(error("Invalid budget period/revision"));
    }
    if let Some(receipt) = replay(w, r, mode == Mode::Opening)? {
        return Ok(PreparedBudget {
            changes: None,
            reads: None,
            receipt,
            opening: mode == Mode::Opening,
            opening_receipt: None,
        });
    }
    if w.production_phase_receipts().values()
        .any(|p| p.request.period == r.period)
        || w.market_phase_receipts().values()
            .any(|p| p.request.period == r.period)
    {
        return Err(error("Budget opening must precede market/production"));
    }
    let mut session = OperationSession::new(w)?;
    session.policies.taxes.extend(r.policies.taxes.clone());
    session.policies.wages.extend(r.policies.wages.clone());
    session.policies.extra.extend(r.policies.extra.clone());
    if !r.policies.priorities.is_empty() {
        session.policies.priorities = r.policies.priorities.clone();
    }
    session.period = Some(r.period.clone());
    validate_service_labor(&session)?;
    let prior = replay(w, r, true)?;
    if mode == Mode::Services && prior.is_none() {
        return Err(error("Service payroll requires authentic budget opening"));
    }
    let opening_receipt;
    let mut receipt = if let Some(p) = prior {
        if p.operations.last().unwrap().meta.get("effective_policies")
            != Some(&json!(session.policies))
        {
            return Err(error("Effective budget policy changed after opening"));
        }
        opening_receipt = None;
        p
    } else {
        reject_orphan_history(w, r)?;
        let mut operations = plan_adventurers(&mut session, &r.period)?;
        let (funding, requirements) = plan_state_funding(&mut session, &r.period)?;
        operations.extend(funding);
        operations.extend(plan_private_funding(&mut session, &r.period)?);
        operations.extend(plan_prior_production_obligations(&mut session, &r.period)?);
        let issuance_by_currency = verify_budget_session(&session, &operations)?;
        let mut p = BudgetPhaseReceipt {
            campaign_id: w.campaign().campaign_id.clone(),
            request: r.clone(),
            operations,
            issuance_by_currency,
            requirements,
        };
        append_boundary(&mut session, &mut p, "opening")?;
        opening_receipt = if mode != Mode::Opening {
            Some(p.clone())
        } else {
            None
        };
        p
    };
    if mode != Mode::Opening {
        let payroll = plan_service_payroll(&mut session, r)?;
        receipt.operations.extend(payroll);
        verify_budget_session(&session, session.operations())?;
        append_boundary(&mut session, &mut receipt, "services")?;
    }
    Ok(PreparedBudget {
        changes: Some(session.finish()?),
        reads: Some(BudgetReadSet::capture(w)),
        receipt,
        opening: mode == Mode::Opening,
        opening_receipt,
    })
}
pub fn apply_budget_cycle(world:&mut WorldState,p:PreparedBudget)->OpResult<BudgetPhaseReceipt> {
    crate::world_transaction::execute_atomic(world,|world|apply_budget_cycle_in(world,p))
}
pub(crate) fn apply_budget_cycle_in(w: &mut crate::world_transaction::WorldTransaction<'_>, p: PreparedBudget) -> OpResult<BudgetPhaseReceipt> {
    if let Some(receipt) = replay(w, &p.receipt.request, p.opening)? {
        return Ok(receipt);
    }
    if w.market_phase_receipts()
        .iter()
        .any(|r| r.request.period == p.receipt.request.period)
        || w.production_phase_receipts()
            .iter()
            .any(|r| r.request.period == p.receipt.request.period)
    {
        return Err(error("Budget opening must precede market/production"));
    }
    if p.reads.as_ref().is_some_and(|reads| !reads.unchanged(w)) {
        return Err(error("Prepared budget consulted rows changed"));
    }
    let changes = p
        .changes
        .ok_or_else(|| error("Budget replay receipt removed"))?;
    w.apply_phase(changes)?;
    if let Some(opening) = p.opening_receipt {
        let key=w.budget_opening_receipts().next_key().map_err(OperationError)?;
        w.put_budget_opening_receipts(key,opening)?;
    }
    if p.opening {
        let key=w.budget_opening_receipts().next_key().map_err(OperationError)?;
        w.put_budget_opening_receipts(key,p.receipt.clone())?;
    } else {
        let key=w.budget_phase_receipts().next_key().map_err(OperationError)?;
        w.put_budget_phase_receipts(key,p.receipt.clone())?;
    }
    Ok(p.receipt)
}

fn reject_orphan_history(w: &dyn WorldRead, r: &BudgetRequest) -> OpResult<()> {
    let financial = [
        format!("payroll:{}:{}:", w.campaign().campaign_id, r.period),
        format!("debtpay:{}:{}:", w.campaign().campaign_id, r.period),
    ];
    let private = [
        format!("budget-funding:{}:", r.period),
        format!("budget-sales-funding:{}:", r.period),
        format!("budget-phase:{}:{}:", w.campaign().campaign_id, r.period),
    ];
    for event in w.committed_operations().values() {
        let v = &event.value;
        let id = event.event_id.as_str().unwrap_or("");
        let kind = v.get("type").and_then(Value::as_str);
        let standard = matches!(kind, Some("payroll" | "blocked" | "debt_payment"))
            && financial.iter().any(|p| id.starts_with(p));
        let internal = kind == Some("budget_funding") && private.iter().any(|p| id.starts_with(p));
        let json_event = match kind {
            Some("adventurer_income") => id.strip_prefix("adventurer-income:"),
            Some("state_budget_funding") => id.strip_prefix("state-budget-funding:"),
            _ => None,
        }
        .and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok())
        .is_some_and(|parts| {
            parts.first().and_then(Value::as_str) == Some(&w.campaign().campaign_id)
                && parts.get(1).and_then(Value::as_str) == Some(&r.period)
        });
        if standard
            || internal
            || json_event
            || kind == Some("budget_phase_boundary") && private.iter().any(|p| id.starts_with(p))
        {
            return Err(error(
                "Orphan budget history requires complete phase receipt",
            ));
        }
    }
    Ok(())
}
pub fn execute_budget_cycle(w:&mut WorldState,r:&BudgetRequest)->OpResult<BudgetPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_budget_cycle_in(w,r))
}
pub(crate) fn execute_budget_cycle_in(w:&mut crate::world_transaction::WorldTransaction<'_>,r:&BudgetRequest)->OpResult<BudgetPhaseReceipt> {
    let p=prepare_budget_cycle(w, r)?;
    apply_budget_cycle_in(w,p)
}
pub fn execute_budget_opening(w:&mut WorldState,r:&BudgetRequest)->OpResult<BudgetPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_budget_opening_in(w,r))
}
pub(crate) fn execute_budget_opening_in(w:&mut crate::world_transaction::WorldTransaction<'_>,r:&BudgetRequest)->OpResult<BudgetPhaseReceipt> {
    let p=prepare(w, r, Mode::Opening)?;
    apply_budget_cycle_in(w,p)
}
pub fn execute_budget_services(w:&mut WorldState,r:&BudgetRequest)->OpResult<BudgetPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_budget_services_in(w,r))
}
pub(crate) fn execute_budget_services_in(w:&mut crate::world_transaction::WorldTransaction<'_>,r:&BudgetRequest)->OpResult<BudgetPhaseReceipt> {
    let p=prepare(w, r, Mode::Services)?;
    apply_budget_cycle_in(w,p)
}
