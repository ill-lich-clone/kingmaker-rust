//! Atomic capacity → release → competitive local/artisan hiring, before budget and production.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    labor_capacity::derive_labor_jobs,
    labor_release::release_employment,
    labor_roster::build_labor_roster,
    local_hiring::{
        plan_artisan_hiring_in, plan_local_hiring_in, HiringScope,
    },
    operation_session::*,
};
use kingmaker_core::{
    labor::*, *,
};
use std::collections::HashMap;
#[derive(Debug, Clone, PartialEq)]
struct ReadSet { hashes: Option<Vec<[u8;32]>> }
impl ReadSet {
    fn capture(w:&dyn WorldRead)->Self {
        let hashes=(|| -> OpResult<Vec<[u8;32]>> { Ok(vec![
            crate::read_guard::fingerprint(&w.provinces())?,
            crate::read_guard::fingerprint(&w.pops())?,
            crate::read_guard::fingerprint(&w.enterprises())?,
            crate::read_guard::fingerprint(&w.stockpiles())?,
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
            crate::read_guard::fingerprint(&w.territory())?,
            crate::read_guard::fingerprint(&w.maintenance_agreements())?,
            crate::read_guard::fingerprint(&w.sale_offers())?,
            crate::read_guard::fingerprint(&w.employment())?,
            crate::read_guard::fingerprint(&w.adventurer_income_rates())?,
            crate::read_guard::fingerprint(&w.institutions())?,
            crate::read_guard::fingerprint(&w.market_construction_catalog())?,
            crate::read_guard::fingerprint(&w.jobs())?,
            crate::read_guard::fingerprint(&w.profiles())?,
            crate::read_guard::fingerprint(&w.species())?,
            crate::read_guard::fingerprint(&w.all_production_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_market_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_budget_phase_receipts())?,
            crate::read_guard::fingerprint(&w.all_budget_opening_receipts())?,
            crate::read_guard::fingerprint(&w.all_labor_phase_receipts())?,
            crate::read_guard::fingerprint(&w.campaign())?,
            crate::read_guard::fingerprint(&w.production_policies())?,
            crate::read_guard::fingerprint(&w.production_catalog())?,
            crate::read_guard::fingerprint(&w.needs())?,
            crate::read_guard::fingerprint(&w.price_calibration())?,
            crate::read_guard::fingerprint(&w.market_parameters())?,
            crate::read_guard::fingerprint(&w.consumer_facility_by_province())?,
            crate::read_guard::fingerprint(&w.budget_profile_id())?,
            crate::read_guard::fingerprint(&w.foundation_budget_policy())?,
            crate::read_guard::fingerprint(&w.institution_rules())?,
            crate::read_guard::fingerprint(&w.pop_mobility())?,
        ]) })().ok();
        Self {hashes}
    }
    fn unchanged(&self,w:&dyn WorldRead)->bool { self.hashes.is_some() && self.hashes==Self::capture(w).hashes }
}
pub struct PreparedLocalLabor {
    reads: Option<ReadSet>,
    delta:Option<crate::world_transaction::PreparedWorldDelta>,
    receipt: LaborPhaseReceipt,
}
fn replay(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<Option<LaborPhaseReceipt>> {
    let rows: Vec<_> = w.all_labor_phase_receipts().select(|p| p.request.period == r.period)?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return Err(error("Duplicate labor receipt"));
    }
    let p = &*rows[0];
    if p.campaign_id != w.campaign().campaign_id || p.request != *r {
        return Err(error("Conflicting labor request"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    for op in &p.operations {
        if !history.matches(op) {
            return Err(error("Missing/conflicting labor history"));
        }
    }
    Ok(Some(p.clone()))
}
fn order(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<()> {
    if w.budget_phase_receipts().values()
        .any(|p| p.request.period == r.period)
        || w.market_phase_receipts().values()
            .any(|p| p.request.period == r.period)
        || w.production_phase_receipts().values()
            .any(|p| p.request.period == r.period)
    {
        return Err(error(
            "Labor opening must precede budget, market and production",
        ));
    }
    Ok(())
}
pub fn prepare_local_labor(w: &dyn WorldRead, r: &LaborRequest) -> OpResult<PreparedLocalLabor> {
    if r.period.is_empty() || r.expected_revision != w.campaign().revision {
        return Err(error("Invalid labor period/revision"));
    }
    if let Some(receipt) = replay(w, r)? {
        return Ok(PreparedLocalLabor {
            reads: None,
            delta:None,
            receipt,
        });
    }
    order(w, r)?;
    for e in w.committed_operations().values() {
        if e.event_id
            .as_str()
            .is_some_and(|id| id.starts_with("hiring-funding:"))
        {
            let payload = &e.event_id.as_str().unwrap()["hiring-funding:".len()..];
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                if value[0].as_str().is_some_and(|p| {
                    p == r.period || p.starts_with(&format!("{}:allocation:", r.period))
                }) {
                    return Err(error(
                        "Orphan hiring history without complete phase receipt",
                    ));
                }
            }
        }
    }
    let mut p=crate::world_transaction::WorldTransaction::new(w);
    p.replace_jobs(derive_labor_jobs(&p)?)?;
    let release = release_employment(&p, &r.period)?;
    let released_fte = release.released_fte;
    let released = release.released.clone();
    let mut blocked = release.blocked.clone();
    release.apply_in(&mut p)?;
    let local = plan_local_hiring_in(&mut p, &r.period, &r.policies,&HiringScope::default())?;
    let mut hires = local.hires;
    blocked.extend(local.blocked);
    let operations = local.operations;
    let artisans = plan_artisan_hiring_in(&mut p, &r.period)?;
    hires.extend(artisans.hires);
    blocked.extend(artisans.blocked);
    build_labor_roster(&p)?;
    if w.pops().len() != p.pops().len()
        || w.pops().values().zip(p.pops().iter()).any(|(a, b)| {
            a.external_id != b.external_id
                || a.size != b.size
                || a.workforce != b.workforce
                || a.province_id != b.province_id
                || a.pop_type_id != b.pop_type_id
                || a.currency_id != b.currency_id
        })
    {
        return Err(error("Local labor changed people or immutable POP rights"));
    }
    let mut session = OperationSession::new(w)?;
    session.period = Some(r.period.clone());
    for op in &operations {
        session.stage_labor(op)?;
    }
    let views = (&p as &dyn WorldRead)
        .derive_enterprise_views(session.accounts(), p.factories().iter(), 0)
        .map_err(OperationError)?;
    let changes = session.finish()?;
    let receipt = LaborPhaseReceipt {
        campaign_id: w.campaign().campaign_id.clone(),
        request: r.clone(),
        released_fte,
        hired_fte: hires.iter().map(|h| h["fte"].as_f64().unwrap()).sum(),
        released,
        hires,
        blocked,
        operations,
    };
    p.discard_forecast_accounts();
    p.apply_phase(changes)?;
    p.replace_enterprises(views)?;
    let key=p.labor_phase_receipts().next_key().map_err(OperationError)?;
    p.put_labor_phase_receipts(key,receipt.clone())?;
    Ok(PreparedLocalLabor {
        reads: Some(ReadSet::capture(w)),
        delta:Some(p.finish()?),
        receipt,
    })
}
pub fn apply_local_labor(w:&mut WorldState,p:PreparedLocalLabor)->OpResult<LaborPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|apply_local_labor_in(w,p))
}
pub(crate) fn apply_local_labor_in(w:&mut crate::world_transaction::WorldTransaction<'_>,p:PreparedLocalLabor)->OpResult<LaborPhaseReceipt> {
    if let Some(receipt)=replay(w,&p.receipt.request)? {return Ok(receipt);}
    order(w,&p.receipt.request)?;
    if p.reads.as_ref().is_none_or(|read|!read.unchanged(w)) {return Err(error("Prepared local labor consulted rows changed"));}
    w.apply_delta(p.delta.ok_or_else(||error("Prepared labor changes removed"))?)?;
    Ok(p.receipt)
}
pub fn execute_local_labor(w:&mut WorldState,r:&LaborRequest)->OpResult<LaborPhaseReceipt> {
    crate::world_transaction::execute_atomic(w,|w|execute_local_labor_in(w,r))
}
pub(crate) fn execute_local_labor_in(w:&mut crate::world_transaction::WorldTransaction<'_>,r:&LaborRequest)->OpResult<LaborPhaseReceipt> {
    let p=prepare_local_labor(w,r)?;
    apply_local_labor_in(w,p)
}
