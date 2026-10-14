//! Validate the actual phase receipts and committed events, not a caller's list of completed names.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    operation_history::operation_evidence,
    operation_session::{canonical_operation, OpResult},
};
use kingmaker_core::{operations::*, week::*, *};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
struct HashWriter(Sha256);
impl std::io::Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn digest<T: Serialize>(value: &T) -> OpResult<String> {
    let mut writer = HashWriter(Sha256::new());
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer
        .0
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
pub(crate) fn boundary_id(w: &dyn WorldRead, r: &NativeWeekRequest) -> String {
    format!("native-week:{}:{}", w.campaign().campaign_id, r.period)
}
pub(crate) fn receipt_digest(p: &NativeWeekReceipt) -> OpResult<String> {
    digest(
        &json!({"campaign_id":p.campaign_id,"request":p.request,"start_date":p.start_date,"end_date":p.end_date,"end_revision":p.end_revision,"phase_order":p.phase_order,"operation_evidence":p.operation_evidence,"registry_evidence":p.registry_evidence,"diagnostics":p.diagnostics,"invariants":p.invariants}),
    )
}
pub(crate) fn registry_evidence(
    w: &dyn WorldRead,
    r: &NativeWeekRequest,
) -> OpResult<Vec<(String, String)>> {
    let mut evidence = vec![];
    macro_rules! rows {
        ($name:literal,$rows:expr,$count:expr) => {{
            let rows = $rows.select(|p| p.request.period == r.period)?;
            if rows.len() != $count
                || rows.iter().any(|p| {
                    p.campaign_id != w.campaign().campaign_id
                        || p.request.expected_revision != r.expected_revision
                })
            {
                return Err(error(concat!("Incomplete native week phase: ", $name)));
            }
            evidence.push(($name.into(), digest(&rows)?));
            rows
        }};
    }
    rows!("budget_opening", w.all_budget_opening_receipts(), 1);
    rows!("budget", w.all_budget_phase_receipts(), 1);
    let mobility = rows!("mobility", w.all_mobility_opening_receipts(), 1);
    if crate::mobility_cycle::replay(w, &mobility[0].request)?.as_ref() != Some(&*mobility[0]) {
        return Err(error("Native week mobility history altered"));
    }
    let cohorts = w.all_mobility_cohort_receipts().select(|p| p.request.expected_revision == r.expected_revision)?;
    evidence.push(("cohorts".into(), digest(&cohorts)?));
    rows!("labor", w.all_labor_phase_receipts(), 1);
    rows!("labor_history", w.all_mobility_history_receipts(), 1);
    rows!("production", w.all_production_phase_receipts(), 1);
    let construction = rows!("construction", w.all_construction_phase_receipts(), 3);
    let mut kinds = HashSet::new();
    if construction.iter().any(|p| !kinds.insert(p.stage as u8)) {
        return Err(error("Duplicate construction stage in native week"));
    }
    let income = rows!("income", w.all_income_phase_receipts(), 2);
    let mut kinds = HashSet::new();
    if income.iter().any(|p| !kinds.insert(p.stage as u8)) {
        return Err(error("Duplicate income stage in native week"));
    }
    let markets = w.all_market_phase_receipts().select(|p| p.request.period == r.period)?;
    let mut kinds = HashSet::new();
    if !(markets.len() == 2 || markets.len() == 3)
        || markets.iter().any(|p| {
            p.campaign_id != w.campaign().campaign_id
                || p.request.expected_revision != r.expected_revision
                || !kinds.insert(p.request.stage as u8)
        })
        || !markets
            .iter()
            .any(|p| p.request.stage == kingmaker_core::market::MarketStage::Maintenance)
        || !markets
            .iter()
            .any(|p| p.request.stage == kingmaker_core::market::MarketStage::ClosingMarket)
    {
        return Err(error("Incomplete native week market"));
    }
    evidence.push(("market".into(), digest(&markets)?));
    let transport = w.all_transport_phase_receipts().select(|p| {
            p.request.period == r.period
                && matches!(
                    p.request.stage,
                    kingmaker_core::transport::TransportStage::Arrivals
                        | kingmaker_core::transport::TransportStage::LocalFoodReserves
                        | kingmaker_core::transport::TransportStage::Dispatch
                        | kingmaker_core::transport::TransportStage::StateProcurement
                )
        })?;
    let mut kinds = HashSet::new();
    if transport.len() != 4
        || transport.iter().any(|p| {
            p.campaign_id != w.campaign().campaign_id
                || p.request.expected_revision != r.expected_revision
                || !kinds.insert(p.request.stage as u8)
        })
    {
        return Err(error("Incomplete native week transport"));
    }
    evidence.push(("transport".into(), digest(&transport)?));
    Ok(evidence)
}
pub fn validate_week_history(
    w: &dyn WorldRead,
    r: &NativeWeekRequest,
) -> OpResult<Option<NativeWeekReceipt>> {
    // A new period is still bound to the last completed interval. Never let a
    // changed campaign header bypass the history merely by choosing another ID.
    let mut previous: Option<(u64,kingmaker_core::calendar::GameDate)> = None;
    let mut periods = HashSet::new();
    for p in w.all_week_phase_receipts().scan() {
        let row=p?;let p=&*row;
        if p.campaign_id != w.campaign().campaign_id
            || !periods.insert(p.request.period.clone())
            || p.request.expected_revision.checked_add(1) != Some(p.end_revision)
            || crate::calendar::add_days(p.start_date, 7)? != p.end_date
            || p.boundary.meta.get("receipt_sha256") != Some(&json!(receipt_digest(p)?))
            || previous.is_some_and(|(revision,date)| {
                p.request.expected_revision != revision || p.start_date != date
            })
        {
            return Err(error("Broken completed native week chain"));
        }
        previous = Some((p.end_revision,p.end_date));
    }
    if let Some((revision,date)) = previous {
        if w.campaign().revision != revision
            || kingmaker_core::calendar::GameDate::from(w.campaign()) != date
        {
            return Err(error(
                "Campaign header does not match completed native week head",
            ));
        }
    }
    if w.all_committed_operations().scan().try_fold(0usize,|n,row|row.map(|o|n+usize::from(o.value["type"]=="native_week_boundary")))?
        != w.all_week_phase_receipts().len()?
    {
        return Err(error("Orphan or missing native week boundary"));
    }
    let rows = w.all_week_phase_receipts().select(|p| p.request.period == r.period)?;
    let id: ExternalId = boundary_id(w, r).into();
    if rows.is_empty() {
        if w.all_committed_operations().get(&id)?.is_some()
        {
            return Err(error("Orphan native week boundary"));
        }
        return Ok(None);
    }
    if rows.len() != 1 || rows[0].request != *r || rows[0].campaign_id != w.campaign().campaign_id {
        return Err(error("Conflicting native week request"));
    }
    let p = &*rows[0];
    if p.end_revision
        != r.expected_revision
            .checked_add(1)
            .ok_or_else(|| error("Week revision overflow"))?
        || crate::calendar::add_days(p.start_date, 7)? != p.end_date
        || w.campaign().revision < p.end_revision
        || (w.campaign().year, w.campaign().month, w.campaign().day)
            < (p.end_date.year, p.end_date.month, p.end_date.day)
        || !p.invariants.production_required_passed()
    {
        return Err(error("Native week date/revision/audit altered"));
    }
    if p.boundary.kind != OperationKind::NativeWeekBoundary
        || p.boundary.event_id != id
        || p.boundary.meta.get("receipt_sha256") != Some(&json!(receipt_digest(p)?))
        || registry_evidence(w, r)? != p.registry_evidence
    {
        return Err(error("Native week receipt or phase registry altered"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let mut seen = HashSet::new();
    if p.operation_evidence.iter().any(|e| {
            !seen.insert(&e.event_id)
                || !history.evidence_matches(e)
        })
        || !history.matches(&p.boundary)
    {
        return Err(error("Native week operation history missing/altered"));
    }
    Ok(Some(p.clone()))
}
