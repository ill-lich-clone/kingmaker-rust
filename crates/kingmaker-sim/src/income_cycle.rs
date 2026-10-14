//! Closed phase: verified evidence, one private native state, all-or-nothing publication.
use kingmaker_core::world_read::WorldRead;
use crate::{
    artisan_choices::plan_artisan_choices,
    artisan_income::plan_artisan_income,
    budget_funding::error,
    income_basis::{validate_income_basis, IncomeOpeningBasis},
    income_invariants::verify_income,
    income_operations::stage_income_operations,
    income_recognition::required_income_operations,
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult, OperationSession},
    producer_income::{effective_income_policies, plan_producer_income},
};
use kingmaker_core::{income::*, operations::*, WorldState};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
pub struct PreparedIncome {
    reads: Option<crate::world_transaction::PrepareProof>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: IncomePhaseReceipt,
}
fn registry_digest(registry: &IncomeRegistryChanges) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(registry)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn input_digest(inputs: &[OperationEvidence]) -> OpResult<String> {
    Ok(Sha256::digest(serde_json::to_vec(inputs)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn budget_opening_inputs(w: &dyn WorldRead, r: &IncomeRequest) -> OpResult<Vec<OperationEvidence>> {
    let opening = w.all_budget_opening_receipts().select(|p| p.request.period == r.period)?;
    if opening.len() > 1 {
        return Err(error("Conflicting budget opening before artisan choices"));
    }
    let Some(p) = opening.first() else {
        return Ok(vec![]);
    };
    if p.campaign_id != w.campaign().campaign_id || p.request.expected_revision != r.expected_revision
    {
        return Err(error("Artisan budget opening revision mismatch"));
    }
    crate::budget_cycle::validate_budget_receipt(w, p)?;
    p.operations.iter().map(operation_evidence).collect()
}
pub(crate) fn replay(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    stage: IncomeStage,
) -> OpResult<Option<IncomePhaseReceipt>> {
    let matches: Vec<_> = w.all_income_phase_receipts().select(|p| p.request.period == r.period && p.stage == stage)?;
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() != 1
        || matches[0].request != *r
        || matches[0].campaign_id != w.campaign().campaign_id
    {
        return Err(error("Conflicting income phase replay"));
    }
    let receipt = &*matches[0];
    if receipt.boundary.kind != OperationKind::IncomePhaseBoundary
        || receipt.boundary.event_id.json()
            != json!(format!(
                "income-phase:{}:{}:{stage:?}",
                w.campaign().campaign_id, r.period
            ))
        || receipt.boundary.meta.get("stage") != Some(&json!(stage))
        || receipt.boundary.meta.get("period") != Some(&json!(r.period))
        || receipt.boundary.meta.get("opening_profile") != Some(&json!(receipt.opening_profile))
        || !receipt.boundary.debits.is_empty()
        || !receipt.boundary.credits.is_empty()
        || !receipt.boundary.stock_changes.is_empty()
        || receipt.boundary.obligation.is_some()
    {
        return Err(error("Invalid income phase boundary"));
    }
    if !receipt.invariants.all_required_passed() {
        return Err(error("Income receipt lacks required audits"));
    }
    if receipt.boundary.meta.get("registry_sha256")
        != Some(&json!(registry_digest(&receipt.registry_changes)?))
    {
        return Err(error("Income registry history altered"));
    }
    if receipt.boundary.meta.get("input_sha256")
        != Some(&json!(input_digest(&receipt.input_evidence)?))
    {
        return Err(error("Income input history altered"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let ops: Vec<_> = receipt
        .operations
        .iter()
        .chain(std::iter::once(&receipt.boundary))
        .collect();
    let mut seen = HashSet::new();
    if ops.iter().any(|op| !seen.insert(&op.event_id)) {
        return Err(error("Duplicate income receipt operations"));
    }
    let proof: Vec<_> = ops
        .iter()
        .map(|op| operation_evidence(op))
        .collect::<OpResult<_>>()?;
    if proof != receipt.operation_evidence
        || ops
            .iter()
            .any(|op| !history.matches(op))
    {
        return Err(error("Missing/conflicting income history"));
    }
    if stage == IncomeStage::ClosingIncome {
        let inputs = required_income_operations(w, r)?;
        if inputs
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<Vec<_>>>()?
            != receipt.input_evidence
        {
            return Err(error("Income prerequisite evidence altered"));
        }
    } else if !receipt.input_evidence.is_empty()
        && budget_opening_inputs(w, r)? != receipt.input_evidence
    {
        return Err(error("Opening income budget evidence altered"));
    }
    Ok(Some(receipt.clone()))
}
fn reject_orphan(w: &dyn WorldRead, r: &IncomeRequest, stage: IncomeStage) -> OpResult<()> {
    let boundary = format!(
        "income-phase:{}:{}:{stage:?}",
        w.campaign().campaign_id, r.period
    );
    let prefixes = if stage == IncomeStage::ClosingIncome {
        vec![
            format!("producer-dividend:{}:", r.period),
            format!("artisan-income:{}:", r.period),
            format!("artisan-choice:{}:", r.period),
        ]
    } else {
        vec![
            format!("artisan-retooling-complete:{}:", r.period),
            format!("artisan-opening-choice:{}:", r.period),
            format!("artisan-retool-capital:{}:", r.period),
            format!("artisan_capital_funding:[\"{}\",", r.period),
            format!("artisan_capital_release:[\"{}\",", r.period),
            format!("artisan-retained-property:[\"{}\",", r.period),
        ]
    };
    if w.committed_operations().values().any(|op| {
        op.event_id.json() == json!(boundary)
            || prefixes
                .iter()
                .any(|p| crate::needs::js_string(&op.event_id.json()).starts_with(p))
    }) {
        return Err(error(
            "Orphan income operation without complete phase receipt",
        ));
    }
    Ok(())
}
fn apply_operations(state: &mut crate::world_transaction::WorldTransaction<'_>, ops: &[EconomicOperation]) -> OpResult<()> {
    if ops.is_empty() {
        return Ok(());
    }
    let mut session = OperationSession::new(state)?;
    stage_income_operations(&mut session, ops)?;
    state.apply_phase(session.finish()?)
}
fn prepare(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    basis: Option<&IncomeOpeningBasis>,
    stage: IncomeStage,
    opening_profile: Option<&kingmaker_core::market::MarketProfile>,
    local:Option<&crate::world_transaction::WorldTransaction<'_>>,
) -> OpResult<PreparedIncome> {
    if let Some(receipt) = replay(w, r, stage)? {
        if receipt.opening_profile.as_ref() != opening_profile {
            return Err(error("Conflicting artisan opening profile"));
        }
        return Ok(PreparedIncome {
            reads: None,
            state: None,
            receipt,
        });
    }
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid income phase revision/period"));
    }
    reject_orphan(w, r, stage)?;
    let input_evidence = if stage == IncomeStage::ClosingIncome {
        validate_income_basis(
            w,
            r,
            basis.ok_or_else(|| error("Income opening basis missing"))?,
        )?;
        required_income_operations(w, r)?
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<Vec<_>>>()?
    } else {
        let evidence = budget_opening_inputs(w, r)?;
        if w.budget_phase_receipts()
            .iter()
            .any(|p| p.request.period == r.period)
            || w.production_phase_receipts()
                .iter()
                .any(|p| p.request.period == r.period)
            || w.market_phase_receipts()
                .iter()
                .any(|p| p.request.period == r.period)
        {
            return Err(error("Artisan opening must precede economic phases"));
        }
        evidence
    };
    let mut state=local.map_or_else(||crate::world_transaction::WorldTransaction::new(w),|parent|parent.child());
    if stage == IncomeStage::ArtisanOpening {
        state.replace_stock_contributions(crate::stock_provenance::capture_stock_provenance(w)?)?;
    }
    let mut operations = vec![];
    let mut diagnostics = vec![];
    if stage == IncomeStage::ClosingIncome {
        let basis = basis.unwrap();
        let artisan = plan_artisan_income(&state, r, basis)?;
        apply_operations(&mut state, &artisan.operations)?;
        operations.extend(artisan.operations);
        diagnostics.extend(artisan.diagnostics);
        state.replace_artisans(artisan.enterprises)?;
        state.replace_stock_contributions(artisan.stock_contributions)?;
        let producer = plan_producer_income(&state, r, basis)?;
        apply_operations(&mut state, &producer.operations)?;
        operations.extend(producer.operations);
        diagnostics.extend(producer.diagnostics);
        for row in producer.history {
            let existing=state.producer_income().keyed().find(|(_,old)|old["owner_id"]==row["owner_id"] && old["currency_id"]==row["currency_id"]);
            if let Some((key,old))=existing {
                let mut next=old.clone();
                next.as_object_mut().ok_or_else(||error("Invalid producer history row"))?.extend(row.as_object().ok_or_else(||error("Invalid calculated producer history"))?.clone());
                state.put_producer_income(key,next)?;
            } else {
                let key=state.producer_income().next_key().map_err(crate::operation_session::OperationError)?;
                state.put_producer_income(key,row)?;
            }
        }
    }
    let choices = plan_artisan_choices(&state, r, stage == IncomeStage::ArtisanOpening)?;
    apply_operations(&mut state, &choices.operations)?;
    operations.extend(choices.operations);
    diagnostics.extend(choices.diagnostics);
    state.replace_artisans(choices.enterprises)?;
    if stage == IncomeStage::ArtisanOpening {
        let capital = crate::artisan_working_capital::plan_artisan_working_capital(
            &state,
            r,
            opening_profile.ok_or_else(|| error("Artisan opening profile missing"))?,
        )?;
        apply_operations(&mut state, &capital.operations)?;
        operations.extend(capital.operations);
        diagnostics.extend(capital.diagnostics);
        state.replace_artisans(capital.enterprises)?;
    }
    let registry_changes = IncomeRegistryChanges {
        producer_income: state.producer_income().iter().cloned().collect::<Vec<_>>(),
        artisans: state.artisans().iter().cloned().collect::<Vec<_>>(),
        stock_contributions: state.stock_contributions().iter().cloned().collect::<Vec<_>>(),
    };
    let boundary: EconomicOperation = serde_json::from_value(
        json!({"event_id":format!("income-phase:{}:{}:{stage:?}",w.campaign().campaign_id,r.period),"type":"income_phase_boundary","period":r.period,"stage":stage,"opening_profile":opening_profile,"input_sha256":input_digest(&input_evidence)?,"registry_sha256":registry_digest(&registry_changes)?}),
    )?;
    apply_operations(&mut state, std::slice::from_ref(&boundary))?;
    let audit_ops: Vec<_> = operations
        .iter()
        .cloned()
        .chain(std::iter::once(boundary.clone()))
        .collect();
    let invariants = verify_income(w, &state, &audit_ops, stage)?;
    let receipt = IncomePhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        stage,
        opening_profile: opening_profile.cloned(),
        effective_policies: effective_income_policies(w, r),
        operation_evidence: audit_ops
            .iter()
            .map(operation_evidence)
            .collect::<OpResult<_>>()?,
        input_evidence,
        operations,
        boundary,
        registry_changes,
        diagnostics,
        invariants,
    };
    let key=state.income_phase_receipts().next_key().map_err(crate::operation_session::OperationError)?;
    state.put_income_phase_receipts(key,receipt.clone())?;
    Ok(PreparedIncome {
        reads: Some(crate::world_transaction::PrepareProof::capture(w,local.map(|parent|parent.local_prepare_token()))?),
        state: Some(state.finish()?),
        receipt,
    })
}
pub fn prepare_income_closing(
    w: &dyn WorldRead,
    r: &IncomeRequest,
    basis: &IncomeOpeningBasis,
) -> OpResult<PreparedIncome> {
    prepare(w, r, Some(basis), IncomeStage::ClosingIncome, None, None)
}
pub fn prepare_artisan_opening(w: &dyn WorldRead, r: &IncomeRequest) -> OpResult<PreparedIncome> {
    prepare(
        w,
        r,
        None,
        IncomeStage::ArtisanOpening,
        Some(&Default::default()),
        None,
    )
}
pub fn apply_income_closing(w: &mut WorldState, p: PreparedIncome) -> OpResult<IncomePhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_income_closing_in(w,p))
}
pub(crate) fn apply_income_closing_in(w: &mut crate::world_transaction::WorldTransaction<'_>, p: PreparedIncome) -> OpResult<IncomePhaseReceipt> {
    if p.receipt.stage != IncomeStage::ClosingIncome {
        return Err(error("Expected prepared closing income"));
    }
    apply_income(w, p)
}
pub fn apply_artisan_opening(
    w: &mut WorldState,
    p: PreparedIncome,
) -> OpResult<IncomePhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_artisan_opening_in(w,p))
}
pub(crate) fn apply_artisan_opening_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedIncome,
) -> OpResult<IncomePhaseReceipt> {
    if p.receipt.stage != IncomeStage::ArtisanOpening {
        return Err(error("Expected prepared artisan opening"));
    }
    apply_income(w, p)
}
fn apply_income(w: &mut crate::world_transaction::WorldTransaction<'_>, p: PreparedIncome) -> OpResult<IncomePhaseReceipt> {
    if let Some(receipt) = replay(w, &p.receipt.request, p.receipt.stage)? {
        if receipt != p.receipt {
            return Err(error("Conflicting prepared income replay"));
        }
        return Ok(receipt);
    }
    p.reads.as_ref().ok_or_else(||error("Income replay receipt removed"))?.validate(w)?;
    let state = p
        .state
        .ok_or_else(|| error("Income replay receipt removed"))?;
    w.apply_delta(state)?;
    Ok(p.receipt)
}
pub fn execute_income_closing(
    w: &mut WorldState,
    r: &IncomeRequest,
    basis: &IncomeOpeningBasis,
) -> OpResult<IncomePhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_income_closing_in(w,r,basis))
}
pub(crate) fn execute_income_closing_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &IncomeRequest,
    basis: &IncomeOpeningBasis,
) -> OpResult<IncomePhaseReceipt> {
    let prepared = prepare(w, r, Some(basis), IncomeStage::ClosingIncome, None, Some(w))?;
    apply_income_closing_in(w, prepared)
}
pub fn execute_artisan_opening(
    w: &mut WorldState,
    r: &IncomeRequest,
) -> OpResult<IncomePhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_artisan_opening_in(w,r))
}
pub(crate) fn execute_artisan_opening_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &IncomeRequest,
) -> OpResult<IncomePhaseReceipt> {
    let prepared = prepare(w, r, None, IncomeStage::ArtisanOpening, Some(&Default::default()), Some(w))?;
    apply_artisan_opening_in(w, prepared)
}
pub fn execute_artisan_opening_with_profile(
    w: &mut WorldState,
    r: &IncomeRequest,
    profile: &kingmaker_core::market::MarketProfile,
) -> OpResult<IncomePhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_artisan_opening_with_profile_in(w,r,profile))
}
pub(crate) fn execute_artisan_opening_with_profile_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &IncomeRequest,
    profile: &kingmaker_core::market::MarketProfile,
) -> OpResult<IncomePhaseReceipt> {
    let prepared = prepare(w, r, None, IncomeStage::ArtisanOpening, Some(profile), Some(w))?;
    apply_artisan_opening_in(w, prepared)
}

