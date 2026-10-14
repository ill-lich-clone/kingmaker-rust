//! One atomic opening: walking arrivals → local/artisan work → professions → departures.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    labor_cycle::execute_local_labor_in,
    migration_cycle::{execute_migration_arrivals_in, execute_migration_departures_in},
    operation_session::*,
    profession_cycle::{execute_profession_changes_in, Reads},
};
use kingmaker_core::{labor::LaborRequest, mobility::*, operations::OperationKind, *};
use std::collections::{HashMap, HashSet};
pub struct PreparedMobilityOpening {
    reads: Option<Reads>,
    state: Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: MobilityOpeningReceipt,
}
pub(crate) fn replay(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<Option<MobilityOpeningReceipt>> {
    let rows: Vec<_> = w.all_mobility_opening_receipts().select(|p| p.request.period == r.period)?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 || rows[0].request != *r || rows[0].campaign_id != w.campaign().campaign_id {
        return Err(error("Conflicting mobility opening request"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    for op in &rows[0].operations {
        if !history.matches(op) {
            return Err(error("Missing/conflicting mobility opening history"));
        }
    }
    let local: Vec<_> = w.all_labor_phase_receipts().select(|p| p.request.period == r.period)?;
    let professions: Vec<_> = w.all_mobility_profession_receipts().select(|p| p.request.period == r.period)?;
    let migration: Vec<_> = w.all_mobility_migration_receipts().select(|p| p.request.period == r.period)?;
    if local != vec![&rows[0].local]
        || professions != vec![&rows[0].professions]
        || migration != vec![&rows[0].arrivals, &rows[0].departures]
    {
        return Err(error("Mobility opening subphase receipt missing"));
    }
    crate::cohort_cycle::validate_cohort_receipts(w, &rows[0].operations, r.expected_revision)?;
    Ok(Some(rows[0].clone()))
}
fn order(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<()> {
    if w.budget_phase_receipts().values()
        .any(|p| p.request.period == r.period)
        || w.market_phase_receipts().values()
            .any(|p| p.request.period == r.period)
        || w.production_phase_receipts().values()
            .any(|p| p.request.period == r.period)
    {
        return Err(error("Mobility opening must precede all economic phases"));
    }
    Ok(())
}
pub fn prepare_mobility_opening(
    w: &dyn WorldRead,
    r: &LaborRequest,
) -> OpResult<PreparedMobilityOpening> {
    if r.expected_revision != w.campaign().revision || r.period.is_empty() {
        return Err(error("Invalid mobility opening revision/period"));
    }
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedMobilityOpening {
            reads: None,
            state: None,
            receipt,
        });
    }
    order(w, r)?;
    if w.labor_phase_receipts().values()
        .any(|p| p.request.period == r.period)
        || w.mobility_profession_receipts().values()
            .any(|p| p.request.period == r.period)
        || w.mobility_migration_receipts().values()
            .any(|p| p.request.period == r.period)
    {
        return Err(error("Orphan mobility subphase without complete opening"));
    }
    let mut state=crate::world_transaction::WorldTransaction::new(w);
    let arrivals = execute_migration_arrivals_in(&mut state, r)?;
    let local = execute_local_labor_in(&mut state, r)?;
    let professions = execute_profession_changes_in(&mut state, r)?;
    let departures = execute_migration_departures_in(&mut state, r)?;
    let operations = arrivals
        .operations
        .iter()
        .chain(&local.operations)
        .chain(&professions.operations)
        .chain(&departures.operations)
        .cloned()
        .collect::<Vec<_>>();
    let mut receipt = MobilityOpeningReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        arrivals,
        local,
        professions,
        departures,
        operations,
        invariants: Default::default(),
    };
    let report = crate::mobility_invariants::check_mobility_operations(w, &state, &receipt);
    if !report.all_required_passed() {
        return Err(error(&format!(
            "Mobility opening invariant failed: {report:?}"
        )));
    }
    receipt.invariants = report;
    let mut session = OperationSession::new(w)?;
    let original: HashSet<_> = w.stockpiles().values().map(|s| &s.external_id).collect();
    let templates: Vec<_> = state.stockpiles()
        .iter()
        .filter(|s| !original.contains(&s.external_id))
        .map(|s| (s.external_id.clone(), s.clone()))
        .collect();
    for op in &receipt.operations {
        match op.kind {
            OperationKind::PopMigrationArrival => {
                session.stage_migration_arrival(op)?;
            }
            OperationKind::PopCohortSplit | OperationKind::PopCohortClaim => {
                session.stage_cohort(op, &templates)?;
            }
            OperationKind::LaborHiringFunding | OperationKind::StateBudgetFunding => {
                session.stage_labor(op)?;
            }
            _ => return Err(error("Unexpected mobility opening operation")),
        }
    }
    let expected: HashMap<_, _> = state.accounts()
        .iter()
        .map(|a| (a.address(), a.balance))
        .collect();
    let real: HashMap<_, _> = session
        .accounts()
        .map(|a| (a.address(), a.balance))
        .collect();
    if expected.len() != state.accounts().len() || real != expected {
        return Err(error("Mobility opening cash unexplained by operations"));
    }
    let stocks: HashMap<_, _> = state.stockpiles()
        .iter()
        .map(|s| (&s.external_id, s))
        .collect();
    let actual: Vec<_> = session.stocks().collect();
    if stocks.len() != state.stockpiles().len()
        || actual.len() != stocks.len()
        || actual.iter().any(|s| {
            stocks.get(&s.external_id).is_none_or(|old| {
                let mut same = (*old).clone();
                same.id = s.id;
                same != **s
            })
        })
    {
        return Err(error("Mobility opening stock/title change unexplained"));
    }
    let original: HashSet<_> = w.pops().values().map(|p| &p.external_id).collect();
    let mut next = session.next_ordinal()?;
    for key in state.pops().keyed().map(|(key,_)|key).collect::<Vec<_>>() {
            let p=state.edit_pop(&key)?;
        if !original.contains(&p.external_id) {
            p.id = PopId(next);
            next = next
                .checked_add(1)
                .ok_or_else(|| error("Mobility POP ordinal overflow"))?;
        }
        p.account_id = p.currency_id.as_ref().and_then(|currency| {
            session
                .account(&AccountAddress {
                    owner_id: p.external_id.clone(),
                    currency_id: currency.clone(),
                    purpose: "personal_savings".into(),
                })
                .map(|a| a.id)
        });
        if p.account_id.is_some() {
            p.account_status = "resolved".into();
        }
    }
    state.replace_enterprises((&state as &dyn WorldRead).derive_enterprise_views(
            session.accounts(),
            state.factories().iter(),
            u64::from(next),
        ).map_err(OperationError)?)?;
    let money = session.finish()?;
    let key=state.mobility_opening_receipts().next_key().map_err(OperationError)?;
    state.put_mobility_opening_receipts(key,receipt.clone())?;
    let mut published=crate::world_transaction::WorldTransaction::new(w);
    published.apply_phase(money)?;
    published.adopt_projection(w,state.finish()?,&["pops","provinces","enterprises","profiles","species","labor_phase_receipts","institution_rules","pop_mobility","capitalist_entities","project_investments","production_beneficiaries","factory_beneficiaries","shipments","mobility_history_receipts","mobility_opening_receipts","mobility_migration_receipts","mobility_profession_receipts","mobility_cohort_receipts","mobility_reference","shipment_recovery_plans","jobs","employment","contracts","resource_operations","factories","artisans","storage_facilities","storage_reservations","goods_reservations","needs","territory","sale_offers","maintenance_agreements"])?;
    Ok(PreparedMobilityOpening {
        reads: Some(Reads::capture(w)),
        state: Some(published.finish()?),
        receipt,
    })
}
pub fn apply_mobility_opening(
    w: &mut WorldState,
    p: PreparedMobilityOpening,
) -> OpResult<MobilityOpeningReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_mobility_opening_in(w,p))
}
pub(crate) fn apply_mobility_opening_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    p: PreparedMobilityOpening,
) -> OpResult<MobilityOpeningReceipt> {
    if let Some(receipt) = replay(w, &p.receipt.request)? {
        return Ok(receipt);
    }
    order(w, &p.receipt.request)?;
    if p.reads.as_ref().is_none_or(|r| !r.unchanged(w)) {
        return Err(error("Prepared mobility opening consulted rows changed"));
    }
    w.apply_delta(p.state.ok_or_else(||error("Prepared mobility state missing"))?)?;
    Ok(p.receipt)
}
pub fn execute_mobility_opening(
    w: &mut WorldState,
    r: &LaborRequest,
) -> OpResult<MobilityOpeningReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_mobility_opening_in(w,r))
}
pub(crate) fn execute_mobility_opening_in(
    w: &mut crate::world_transaction::WorldTransaction<'_>,
    r: &LaborRequest,
) -> OpResult<MobilityOpeningReceipt> {
    let p = prepare_mobility_opening(w, r)?;
    apply_mobility_opening_in(w, p)
}
