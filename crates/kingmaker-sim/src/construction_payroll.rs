//! Actual builder wages are private, atomic and justified by native opening history.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    construction_labor::plan_construction_labor,
    construction_operations::stage_construction_operations,
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult, OperationSession},
};
use kingmaker_core::{construction::*, operations::*, production::ProductionRequest, WorldState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedConstructionPayroll {
    reads: Option<crate::world_transaction::PrepareProof>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: ConstructionPhaseReceipt,
}
fn digest(v: &Value) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(v)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn boundary(w: &dyn WorldRead, r: &ConstructionRequest) -> String {
    format!(
        "construction-phase:{}:{}:payroll",
        w.campaign().campaign_id, r.period
    )
}
fn inputs(w: &dyn WorldRead, r: &ConstructionRequest) -> OpResult<Vec<OperationEvidence>> {
    let investment = crate::investments::replay(w, r)?
        .ok_or_else(|| error("Construction payroll requires native investment opening"))?;
    Ok(investment
        .input_evidence
        .into_iter()
        .chain(investment.operation_evidence)
        .collect())
}
pub(crate) fn replay(w: &dyn WorldRead, r: &ConstructionRequest) -> OpResult<Option<ConstructionPhaseReceipt>> {
    let matches: Vec<_> = w.all_construction_phase_receipts().select(|p| p.request.period == r.period && p.stage == ConstructionStage::Payroll)?;
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() != 1
        || matches[0].request != *r
        || matches[0].campaign_id != w.campaign().campaign_id
    {
        return Err(error("Conflicting construction payroll receipt"));
    }
    let p = &*matches[0];
    if !p.invariants.all_required_passed() || p.input_evidence != inputs(w, r)? {
        return Err(error("Construction payroll prerequisite history altered"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut ids = HashSet::new();
    let evidence = p
        .operations
        .iter()
        .map(operation_evidence)
        .collect::<OpResult<Vec<_>>>()?;
    if p.operation_evidence != evidence
        || p.operations.iter().any(|op| {
            !ids.insert(&op.event_id)
                || !history.matches(op)
        })
    {
        return Err(error("Missing or altered construction payroll history"));
    }
    let op = p
        .operations
        .last()
        .ok_or_else(|| error("Construction payroll boundary missing"))?;
    if op.kind != OperationKind::ConstructionPhaseBoundary
        || op.event_id.json() != json!(boundary(w, r))
        || op.meta.get("stage") != Some(&json!("payroll"))
        || op.meta.get("period") != Some(&json!(r.period))
        || op.meta.get("registry_sha256") != Some(&json!(digest(&p.registry_changes)?))
        || !op.debits.is_empty()
        || !op.credits.is_empty()
        || !op.stock_changes.is_empty()
        || op.obligation.is_some()
    {
        return Err(error("Construction payroll registry history altered"));
    }
    Ok(Some(p.clone()))
}
pub fn prepare_construction_payroll(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
) -> OpResult<PreparedConstructionPayroll> {
    prepare_construction_payroll_guarded(w,r,None)
}
fn prepare_construction_payroll_guarded(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedConstructionPayroll> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedConstructionPayroll {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid construction payroll revision or period"));
    }
    let input_evidence = inputs(w, r)?;
    if w.production_phase_receipts()
        .iter()
        .any(|p| p.request.period == r.period)
        || w.market_phase_receipts()
            .iter()
            .any(|p| p.request.period == r.period && p.request.stage==kingmaker_core::market::MarketStage::ClosingMarket)
    {
        return Err(error(
            "Builder payroll must precede producer production and market",
        ));
    }
    if w.committed_operations().iter().any(|op| {
        op.event_id.json() == json!(boundary(w, r))
            || op.value["type"] == json!("payroll")
                && op.value["period"] == json!(r.period)
                && op.value["activity_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with(&format!("construction:{}:", r.period)))
    }) {
        return Err(error("Orphan builder payroll history"));
    }
    let plan = plan_construction_labor(w, r)?;
    let request = ProductionRequest {
        expected_revision: r.expected_revision,
        period: r.period.clone(),
        policies: r.policies.clone(),
        profile: Default::default(),
        maintenance_receipts: vec![],
    };
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    let mut session = OperationSession::new(&state)?;
    session.configure(&request)?;
    let paid =
        crate::production_payroll::pay_construction_labor(&mut session, &request, &plan.requests)?;
    let mut operations = paid
        .into_iter()
        .map(|p| {
            serde_json::from_value::<EconomicOperation>(serde_json::to_value(p)?)
                .map_err(crate::operation_session::OperationError::from)
        })
        .collect::<OpResult<Vec<_>>>()?;
    let changes = session.finish()?;
    state.apply_phase(changes)?;
    let registry_changes = json!({"payroll_requests":plan.requests});
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":boundary(w,r),"type":"construction_phase_boundary","period":r.period,"stage":"payroll","registry_sha256":digest(&registry_changes)?}),
    )?;
    let mut session = OperationSession::new(&state)?;
    stage_construction_operations(&mut session, std::slice::from_ref(&op))?;
    let changes = session.finish()?;
    state.apply_phase(changes)?;
    operations.push(op);
    let invariants = crate::investment_invariants::verify_investment(w, &state, &operations)?;
    let receipt = ConstructionPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        stage: ConstructionStage::Payroll,
        operation_evidence: operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence,
        operations,
        registry_changes,
        decisions: vec![],
        diagnostics: plan.blocked,
        invariants,
    };
    let key=state.construction_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_construction_phase_receipts(key,receipt.clone())?;
    Ok(PreparedConstructionPayroll {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn apply_construction_payroll(
    w: &mut WorldState,
    p: PreparedConstructionPayroll,
) -> OpResult<ConstructionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_construction_payroll_in(w,p))
}
pub(crate) fn apply_construction_payroll_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedConstructionPayroll,
) -> OpResult<ConstructionPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p
            .state
            .ok_or_else(|| error("Builder payroll state missing"))?)?;
    } else if replay(w, &p.receipt.request)?.as_ref() != Some(&p.receipt) {
        return Err(error("Prepared builder replay changed"));
    }
    Ok(p.receipt)
}
pub fn execute_construction_payroll(
    w: &mut WorldState,
    r: &ConstructionRequest,
) -> OpResult<ConstructionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_construction_payroll_in(w,r))
}
pub(crate) fn execute_construction_payroll_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ConstructionRequest,
) -> OpResult<ConstructionPhaseReceipt> {
    let p = prepare_construction_payroll_guarded(w, r, Some(w))?;
    apply_construction_payroll_in(w, p)
}
