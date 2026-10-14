//! Closed cancellation returns real cash and preserves the investors' goods/cargo rights.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    construction_operations::stage_construction_operations,
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult, OperationSession},
};
use kingmaker_core::{construction::*, operations::*, WorldState};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedCancellation {
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
fn phase_request(r: &ConstructionCancellationRequest) -> ConstructionRequest {
    ConstructionRequest {
        expected_revision: r.expected_revision,
        period: crate::needs::js_string(&r.event_id.json()),
        policies: Default::default(),
    }
}
fn validate_proof(w: &dyn WorldRead, p: &ConstructionPhaseReceipt) -> OpResult<()> {
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let proof = p
        .operations
        .iter()
        .map(operation_evidence)
        .collect::<OpResult<Vec<_>>>()?;
    let mut ids = HashSet::new();
    if p.campaign_id != w.campaign().campaign_id
        || !p.invariants.all_required_passed()
        || proof != p.operation_evidence
        || p.operations.iter().any(|o| {
            !ids.insert(&o.event_id)
                || !history.matches(o)
        })
    {
        return Err(error("Missing or altered cancellation/project history"));
    }
    for e in &p.input_evidence {
        let value = history
            .get(&e.event_id)?
            .ok_or_else(|| error("Missing cancellation prerequisite history"))?;
        let op: EconomicOperation = serde_json::from_value((*value).clone())?;
        if operation_evidence(&op)? != *e {
            return Err(error("Altered cancellation prerequisite history"));
        }
    }
    let boundary = p
        .operations
        .last()
        .ok_or_else(|| error("Project boundary missing"))?;
    if boundary.kind != OperationKind::ConstructionPhaseBoundary
        || boundary.meta.get("registry_sha256") != Some(&json!(digest(&p.registry_changes)?))
    {
        return Err(error("Cancellation/project registry proof altered"));
    }
    Ok(())
}
fn replay(
    w: &dyn WorldRead,
    r: &ConstructionCancellationRequest,
) -> OpResult<Option<ConstructionPhaseReceipt>> {
    let rows = w.all_construction_phase_receipts().select(|p| {
            p.stage == ConstructionStage::Cancellation
                && p.request.period == crate::needs::js_string(&r.event_id.json())
        })?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 || rows[0].request != phase_request(r) || rows[0].decisions != vec![json!(r)]
    {
        return Err(error("Conflicting cancellation identity"));
    }
    let p = &*rows[0];
    validate_proof(w, p)?;
    let actual = w.construction_projects().values()
        .find(|p| p.id == r.project_id)
        .ok_or_else(|| error("Cancelled project lost"))?;
    if actual.status != "cancelled"
        || actual.extra.get("cancellation")
            != Some(&json!({"event_id":r.event_id,"reason":r.reason}))
    {
        return Err(error("Cancelled project identity altered"));
    }
    if serde_json::to_value(actual)? != p.registry_changes["project"] {
        return Err(error("Cancelled project rights/claims altered"));
    }
    Ok(Some(p.clone()))
}
pub fn prepare_cancellation(
    w: &dyn WorldRead,
    r: &ConstructionCancellationRequest,
) -> OpResult<PreparedCancellation> {
    prepare_cancellation_guarded(w,r,None)
}
fn prepare_cancellation_guarded(
    w: &dyn WorldRead,
    r: &ConstructionCancellationRequest,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedCancellation> {
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedCancellation {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.expected_revision != w.campaign().revision
        || r.reason.is_empty()
        || crate::needs::js_string(&r.event_id.json()).is_empty()
    {
        return Err(error("Invalid cancellation revision/event/reason"));
    }
    let index = w.construction_projects()
        .iter()
        .position(|p| p.id == r.project_id)
        .ok_or_else(|| error("Cancellation project missing"))?;
    let p = serde_json::to_value(&w.construction_projects().at(index).expect("validated row position"))?;
    if matches!(p["status"].as_str(), Some("completed" | "cancelled")) {
        return Err(error("Project cannot be cancelled again"));
    }
    let owner = crate::transport_validation::actor(&p["owner_id"])?;
    let shares = crate::construction_rights::cancellation_shares(w, &p)?;
    let refund = crate::construction_rights::plan_project_refunds(
        w,
        &p,
        &crate::needs::js_string(&r.event_id.json()),
    )?;
    let mut input_evidence = vec![];
    for contribution in w.project_investments()
        .iter()
        .filter(|i| i["project_id"] == p["id"])
    {
        if let Some(event) = contribution.get("event_id") {
            let op = w.committed_operations()
                .iter()
                .find(|o| o.event_id.json() == *event)
                .ok_or_else(|| error("Investor payment history missing"))?;
            let op: EconomicOperation = serde_json::from_value(op.value.clone())?;
            if op.kind != OperationKind::CapitalistInvestment
                || op.meta.get("project_id") != Some(&p["id"])
                || op.meta.get("investor_id") != Some(&contribution["investor_id"])
                || op.debits.len() != 1
                || op.debits[0].owner_id.json() != contribution["owner_id"]
                || op.debits[0].currency_id.json() != contribution["currency_id"]
                || op.debits[0].purpose
                    != contribution["source_account_purpose"]
                        .as_str()
                        .unwrap_or("")
                || op.debits[0].amount_minor
                    != serde_json::from_value::<kingmaker_core::Money>(
                        contribution["amount_minor"].clone(),
                    )?
            {
                return Err(error("Investor payment history conflicts"));
            }
            input_evidence.push(operation_evidence(&op)?);
        }
    }
    for phase in &w.construction_phase_receipts() {
        if phase.stage != ConstructionStage::Cancellation
            && phase.operations.iter().any(|op| {
                op.meta.get("project_id") == Some(&p["id"])
                    || op
                        .debits
                        .iter()
                        .chain(&op.credits)
                        .any(|leg| leg.owner_id == owner)
            })
        {
            validate_proof(w, phase)?;
            input_evidence.extend(phase.operation_evidence.clone());
            input_evidence.extend(phase.input_evidence.clone());
        }
    }
    let mut unique = HashSet::new();
    input_evidence.retain(|p| unique.insert(p.event_id.clone()));
    let goods=w.stockpiles().iter().filter(|s|s.owner_id==owner&&s.quantity>0.).map(|s|json!({"stock_id":s.external_id,"goods_id":s.goods.external_id(),"amount":s.quantity,"province_id":s.province_id.0,"shares":shares})).collect::<Vec<_>>();
    let cargo=w.shipments().iter().filter(|s|s["buyer_id"]==owner.json()&&s["remaining_quantity"].as_f64().is_some_and(|n|n>0.)).map(|s|json!({"shipment_id":s["id"],"goods_id":s["goods_id"],"amount":s["remaining_quantity"],"province_id":s["to_province_id"],"currency_id":s["currency_id"],"shares":shares})).collect::<Vec<_>>();
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    let mut project = p;
    project["status"] = json!("cancelled");
    project["place_reserved"] = json!(false);
    project["cancellation"] = json!({"event_id":r.event_id,"reason":r.reason});
    project["remaining_goods_claims"] = json!(goods);
    project["remaining_cargo_claims"] = json!(cargo);
    let key=state.construction_projects().keyed().nth(index).ok_or_else(||error("Project disappeared"))?.0;
    state.put_construction_projects(key,serde_json::from_value(project.clone())?)?;
    let registry_changes = json!({"project":project});
    let mut operations = refund.operations;
    operations.push(serde_json::from_value(
        json!({"event_id":r.event_id,"type":"construction_cancelled","project_id":r.project_id}),
    )?);
    operations.push(serde_json::from_value(json!({"event_id":format!("cancellation-boundary:{}:{}",w.campaign().campaign_id,crate::needs::js_string(&r.event_id.json())),"type":"construction_phase_boundary","stage":"cancellation","registry_sha256":digest(&registry_changes)?}))?);
    let mut session = OperationSession::new(&state)?;
    stage_construction_operations(&mut session, &operations)?;
    let changes = session.finish()?;
    state.apply_phase(changes)?;
    let invariants = crate::construction_invariants::verify_construction(w, &state, &operations)?;
    let receipt = ConstructionPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: phase_request(r),
        stage: ConstructionStage::Cancellation,
        operation_evidence: operations
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence,
        operations,
        registry_changes,
        decisions: vec![json!(r)],
        diagnostics: vec![],
        invariants,
    };
    let key=state.construction_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_construction_phase_receipts(key,receipt.clone())?;
    Ok(PreparedCancellation {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn apply_cancellation(
    w: &mut WorldState,
    p: PreparedCancellation,
) -> OpResult<ConstructionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_cancellation_in(w,p))
}
pub(crate) fn apply_cancellation_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedCancellation,
) -> OpResult<ConstructionPhaseReceipt> {
    if let Some(reads) = p.reads {
        reads.validate(w)?;
        w.apply_delta(p.state.ok_or_else(|| error("Cancellation state missing"))?)?;
    } else if replay(
        w,
        &serde_json::from_value::<ConstructionCancellationRequest>(p.receipt.decisions[0].clone())?,
    )?
    .as_ref()
        != Some(&p.receipt)
    {
        return Err(error("Cancellation replay changed"));
    }
    Ok(p.receipt)
}
pub fn execute_cancellation(
    w: &mut WorldState,
    r: &ConstructionCancellationRequest,
) -> OpResult<ConstructionPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_cancellation_in(w,r))
}
pub(crate) fn execute_cancellation_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &ConstructionCancellationRequest,
) -> OpResult<ConstructionPhaseReceipt> {
    let prepared = prepare_cancellation_guarded(w, r, Some(w))?;
    apply_cancellation_in(w, prepared)
}
