//! Prepared construction has exact native paid-labor/production history and one publication.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    construction_operations::stage_construction_operations,
    operation_history::{operation_evidence, validate_production_history},
    operation_session::{canonical_operation, OpResult, OperationSession},
};
use kingmaker_core::{construction::*, operations::*, WorldState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedConstruction {
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
        "construction-phase:{}:{}:progress",
        w.campaign().campaign_id, r.period
    )
}
fn inputs(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
) -> OpResult<(Vec<OperationEvidence>, Vec<EconomicOperation>)> {
    let payroll = crate::construction_payroll::replay(w, r)?
        .ok_or_else(|| error("Construction requires native paid builder phase"))?;
    let production: Vec<_> = w.all_production_phase_receipts().select(|p| p.request.period == r.period)?;
    if production.len() != 1 || production[0].request.expected_revision != r.expected_revision {
        return Err(error("Construction requires native producer phase"));
    }
    let prod = validate_production_history(w, &production[0])?;
    let mut evidence = payroll.input_evidence;
    evidence.extend(payroll.operation_evidence);
    evidence.extend(
        prod.iter()
            .map(operation_evidence)
            .collect::<OpResult<Vec<_>>>()?,
    );
    Ok((evidence, payroll.operations))
}
fn registry(w: &dyn WorldRead) -> Value {
    json!({"projects":w.construction_projects(),"factories":w.factories(),"expansions":w.expansions(),"completed":w.completed_construction(),"storage_facilities":w.storage_facilities()})
}
pub(crate) fn replay(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
) -> OpResult<Option<ConstructionPhaseReceipt>> {
    let matches: Vec<_> = w.all_construction_phase_receipts().select(|p| p.stage == ConstructionStage::Progress && p.request.period == r.period)?;
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() != 1
        || matches[0].request != *r
        || matches[0].campaign_id != w.campaign().campaign_id
    {
        return Err(error("Conflicting construction receipt"));
    }
    let p = &*matches[0];
    if !p.invariants.all_required_passed() || p.input_evidence != inputs(w, r)?.0 {
        return Err(error("Construction prerequisite evidence altered"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut ids = HashSet::new();
    let proof = p
        .operations
        .iter()
        .map(operation_evidence)
        .collect::<OpResult<Vec<_>>>()?;
    if proof != p.operation_evidence
        || p.operations.iter().any(|op| {
            !ids.insert(&op.event_id)
                || !history.matches(op)
        })
    {
        return Err(error("Missing or altered construction history"));
    }
    let op = p
        .operations
        .last()
        .ok_or_else(|| error("Construction boundary missing"))?;
    if op.kind != OperationKind::ConstructionPhaseBoundary
        || op.event_id.json() != json!(boundary(w, r))
        || op.meta.get("registry_sha256") != Some(&json!(digest(&p.registry_changes)?))
        || !op.debits.is_empty()
        || !op.credits.is_empty()
        || !op.stock_changes.is_empty()
    {
        return Err(error("Construction registry history altered"));
    }
    Ok(Some(p.clone()))
}
pub fn prepare_construction(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
) -> OpResult<PreparedConstruction> {
    prepare_construction_guarded(w,r,None)
}
fn prepare_construction_guarded(
    w: &dyn WorldRead,
    r: &ConstructionRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedConstruction> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedConstruction {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid construction period or revision"));
    }
    let (input_evidence, paid) = inputs(w, r)?;
    if w.market_phase_receipts().iter().any(|p| {
        p.request.period == r.period
            && p.request.stage == kingmaker_core::market::MarketStage::ClosingMarket
    }) {
        return Err(error("Construction must precede closing market"));
    }
    if w.committed_operations().iter().any(|op| {
        op.event_id.json() == json!(boundary(w, r))
            || crate::needs::js_string(&op.event_id.json()).starts_with(&format!(
                "construction:{}:{}:",
                w.campaign().campaign_id, r.period
            ))
    }) {
        return Err(error("Orphan construction history"));
    }
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    let (mut operations, diagnostics) = crate::construction::progress(&mut state, r, &paid)?;
    state.refresh_enterprise_views()?;
    let registry_changes = registry(&state);
    let op: EconomicOperation = serde_json::from_value(
        json!({"event_id":boundary(w,r),"type":"construction_phase_boundary","period":r.period,"stage":"progress","registry_sha256":digest(&registry_changes)?}),
    )?;
    let mut session = OperationSession::new(&state)?;
    stage_construction_operations(&mut session, std::slice::from_ref(&op))?;
    let changes = session.finish()?;
    state.apply_phase(changes)?;
    operations.push(op);
    let invariants = crate::construction_invariants::verify_construction(w, &state, &operations)?;
    let receipt = ConstructionPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        stage: ConstructionStage::Progress,
        operation_evidence: operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence,
        operations,
        registry_changes,
        decisions: vec![],
        diagnostics,
        invariants,
    };
    let key=state.construction_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_construction_phase_receipts(key,receipt.clone())?;
    Ok(PreparedConstruction {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn apply_construction(
    w: &mut WorldState,
    p: PreparedConstruction,
) -> OpResult<ConstructionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_construction_in(w,p))
}
pub(crate) fn apply_construction_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedConstruction,
) -> OpResult<ConstructionPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p.state.ok_or_else(|| error("Construction state missing"))?)?;
    } else if replay(w, &p.receipt.request)?.as_ref() != Some(&p.receipt) {
        return Err(error("Prepared construction replay changed"));
    }
    Ok(p.receipt)
}
pub fn execute_construction(
    w: &mut WorldState,
    r: &ConstructionRequest,
) -> OpResult<ConstructionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_construction_in(w,r))
}
pub(crate) fn execute_construction_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ConstructionRequest,
) -> OpResult<ConstructionPhaseReceipt> {
    let p = prepare_construction_guarded(w, r, Some(w))?;
    apply_construction_in(w, p)
}
