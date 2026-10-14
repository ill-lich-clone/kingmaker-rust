//! Evidence for each newly committed operation, never a complete world digest.
use kingmaker_core::world_read::WorldRead;
use crate::{
    budget_funding::error,
    operation_session::{canonical_operation, OpResult},
};
use kingmaker_core::{
    operations::{EconomicOperation, OperationEvidence, OperationKind},
    production::ProductionPhaseReceipt,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

pub(crate) fn operation_evidence(op: &EconomicOperation) -> OpResult<OperationEvidence> {
    let body = serde_json::to_vec(&canonical_operation(op)?)?;
    let sha256 = Sha256::digest(body)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(OperationEvidence {
        event_id: op.event_id.clone(),
        sha256,
    })
}
pub(crate) fn validate_production_history(
    w: &dyn WorldRead,
    r: &ProductionPhaseReceipt,
) -> OpResult<Vec<EconomicOperation>> {
    if r.campaign_id != w.campaign().campaign_id {
        return Err(error("Production evidence campaign mismatch"));
    }
    let history = crate::operation_history::CheckedHistory::new(w)?;
    let ids = r.event_ids.iter().collect::<HashSet<_>>();
    let evidence = r
        .operation_evidence
        .iter()
        .map(|e| (&e.event_id, e))
        .collect::<HashMap<_, _>>();
    if ids.len() != r.event_ids.len()
        || evidence.len() != r.operation_evidence.len()
        || ids != evidence.keys().copied().collect()
    {
        return Err(error("Missing/duplicate production operation evidence"));
    }
    let mut result = vec![];
    for id in &r.event_ids {
        let v = history
            .get(id)?
            .ok_or_else(|| error("Required production operation missing"))?;
        let op: EconomicOperation = serde_json::from_value(v.into_owned())?;
        if op.event_id != *id || operation_evidence(&op)? != *evidence[id] {
            return Err(error("Required production operation body altered"));
        }
        result.push(op);
    }
    let mut payroll_ids = HashSet::new();
    for paid in &r.payroll {
        if !payroll_ids.insert(&paid.event_id) {
            return Err(error("Duplicate production payroll receipt"));
        }
        let Some(op) = result.iter().find(|op| op.event_id == paid.event_id) else {
            return Err(error("Required production payroll event missing"));
        };
        if op.kind != OperationKind::Payroll
            || serde_json::from_value::<kingmaker_core::production::PayrollReceipt>(
                canonical_operation(op)?,
            )? != *paid
        {
            return Err(error("Required production payroll evidence altered"));
        }
    }
    if result
        .iter()
        .any(|op| op.kind == OperationKind::Payroll && !payroll_ids.contains(&op.event_id))
    {
        return Err(error("Unreceipted production payroll"));
    }
    Ok(result)
}

/// The archive has a unique disk index; only current unpublished keys need a RAM set.
pub(crate) fn validate_active_history_keys(w:&dyn WorldRead)->OpResult<()> {
    let mut ids=HashSet::new();
    for row in w.committed_operations().values() {
        crate::run_control::work("active_history_keys")?;
        if !ids.insert(&row.event_id) {return Err(error("Duplicate committed history identity"));}
        if let Some(archive)=w.history() {
            if archive.reader.get(&kingmaker_core::history::HistoryKey{namespace:"committed_operations".into(),id:row.event_id.clone()})?.is_some() {
                return Err(error("Duplicate active/archive history identity"));
            }
        }
    }
    Ok(())
}

/// Verifies only consulted operation bodies. Historical indexes stay in the archive.
pub(crate) struct CheckedHistory<'a> {world:&'a dyn WorldRead}
impl<'a> CheckedHistory<'a> {
    pub(crate) fn new(world:&'a dyn WorldRead)->OpResult<Self> {validate_active_history_keys(world)?;Ok(Self{world})}
    pub(crate) fn get(&self,id:&kingmaker_core::ExternalId)->OpResult<Option<std::borrow::Cow<'a,serde_json::Value>>> {
        use kingmaker_core::history_view::HistoryRow;
        let Some(row)=self.world.all_committed_operations().get(id)? else {return Ok(None);};
        if &row.event_id!=id {return Err(error("Archived operation identity mismatch"));}
        Ok(Some(match row {
            HistoryRow::Borrowed(row)=>std::borrow::Cow::Borrowed(&row.value),
            HistoryRow::Shared(row)=>std::borrow::Cow::Owned(std::sync::Arc::try_unwrap(row).unwrap_or_else(|r|(*r).clone()).value),
        }))
    }
    // Any read/canonicalization error is a failed proof, never evidence of absence or success.
    pub(crate) fn matches(&self,op:&EconomicOperation)->bool {
        match (self.get(&op.event_id),canonical_operation(op)) {
            (Ok(Some(value)),Ok(expected))=>*value==expected,
            _=>false,
        }
    }
    pub(crate) fn evidence_matches(&self,e:&OperationEvidence)->bool {
        let Ok(Some(value))=self.get(&e.event_id) else {return false;};
        serde_json::from_value::<EconomicOperation>(value.into_owned()).ok().and_then(|op|operation_evidence(&op).ok()).as_ref()==Some(e)
    }
}
