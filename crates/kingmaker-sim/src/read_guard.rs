//! Content guards include absent keys and full selection membership, without payload copies.
use crate::operation_session::{OpResult, OperationError};
use kingmaker_core::row_store::RowRead;
use serde::{Serialize, ser::SerializeSeq};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, hash::Hash, io::Write};
pub(crate) type Fingerprint = [u8; 32];
pub(crate) fn fingerprint(value: &impl Serialize) -> OpResult<Fingerprint> {
kingmaker_core::diagnostics::measure("guard.serialize", kingmaker_core::diagnostics::Visibility::Detail, || {
    struct Writer(Sha256,u64,usize,usize);
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            // Bound both tiny serde fragments and single large payloads.
            for chunk in bytes.chunks(64*1024) {
                self.2+=chunk.len(); self.3+=1;
                if self.2>=64*1024 || self.3>=4096 {
                    crate::run_control::check("read_guard").map_err(std::io::Error::other)?;
                    self.2=0;self.3=0;
                }
                self.0.update(chunk);
            }
            self.1=self.1.saturating_add(bytes.len() as u64); Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }
    crate::run_control::check("read_guard")?;
    let mut writer = Writer(Sha256::new(),0,0,0);
    let result=serde_json::to_writer(&mut writer, value);
    kingmaker_core::diagnostics::add(kingmaker_core::diagnostics::Metric::HashCalls,1);
    kingmaker_core::diagnostics::add(kingmaker_core::diagnostics::Metric::HashBytes,writer.1);
    result?;
    Ok(writer.0.finalize().into())

})
}
pub(crate) fn selection<K: Serialize, T: Serialize>(rows: &dyn RowRead<K,T>) -> OpResult<Fingerprint> {
    selection_uncached(rows)
}
pub(crate) fn selection_uncached<K: Serialize, T: Serialize>(rows: &dyn RowRead<K,T>) -> OpResult<Fingerprint> {
    struct Rows<'a,K,T>(&'a dyn RowRead<K,T>);
    impl<K: Serialize,T: Serialize> Serialize for Rows<'_,K,T> {
        fn serialize<S: serde::Serializer>(&self, serializer:S) -> Result<S::Ok,S::Error> {
            let mut seq = serializer.serialize_seq(None)?;
            for row in self.0.iter() { seq.serialize_element(&row)?; }
            seq.end()
        }
    }
    fingerprint(&Rows(rows))
}
#[derive(Clone)]
pub(crate) struct RowGuards<K> {
    values: HashMap<K,Fingerprint>,
    selection: Option<Fingerprint>,
    pub error: Option<String>,
}
impl<K> Default for RowGuards<K> {
    fn default() -> Self { Self { values:HashMap::new(), selection:None, error:None } }
}
impl<K: Clone+Eq+Hash+Serialize> RowGuards<K> {
    pub fn read<T:Serialize>(&mut self, base:&dyn RowRead<K,T>, key:&K) {
        if self.selection.is_some() || self.values.contains_key(key) { return; }
        match fingerprint(&base.get(key)) {
            Ok(hash) => { self.values.insert(key.clone(),hash); },
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    pub fn scan<T:Serialize>(&mut self, base:&dyn RowRead<K,T>) {
        if self.selection.is_some() { return; }
        match selection(base) {
            Ok(hash) => { self.selection = Some(hash); self.values.clear(); },
            Err(e) => self.error = Some(e.to_string()),
        }
    }
    // Only transaction-owned caches reach this private path. External RowRead has no cache hook.
    pub fn scan_owned<T:Serialize>(&mut self,base:&dyn RowRead<K,T>,cache:&std::cell::OnceCell<Result<Fingerprint,String>>) {
        if self.selection.is_some() {return;}
        match cache.get_or_init(||selection(base).map_err(|e|e.to_string())) {
            Ok(hash)=>{self.selection=Some(*hash);self.values.clear();},
            Err(e)=>self.error=Some(e.clone()),
        }
    }
    pub fn validate_owned<T:Serialize>(&self,base:&dyn RowRead<K,T>,cache:&std::cell::OnceCell<Result<Fingerprint,String>>)->OpResult<()> {
        self.validate_with_cache(base,Some(cache))
    }
    pub fn validate<T:Serialize>(&self, base:&dyn RowRead<K,T>) -> OpResult<()> {
        self.validate_with_cache(base,None)
    }
    fn validate_with_cache<T:Serialize>(&self,base:&dyn RowRead<K,T>,cache:Option<&std::cell::OnceCell<Result<Fingerprint,String>>>)->OpResult<()> {
        if let Some(e) = &self.error { return Err(OperationError(e.clone())); }
        if let Some(hash) = self.selection {
            let actual=match cache {Some(cache)=>cache.get_or_init(||selection(base).map_err(|e|e.to_string())).clone().map_err(OperationError)?,None=>selection(base)?};
            if actual != hash { return Err(OperationError("Stale registry selection".into())); }
        } else {
            for (key, hash) in &self.values {
                if fingerprint(&base.get(key))? != *hash { return Err(OperationError("Stale row or absent identity".into())); }
            }
        }
        Ok(())
    }
}

// Preserves legacy whole-state prepare/apply guards without retaining payloads.
pub(crate) fn world_fingerprint(w:&dyn kingmaker_core::world_read::WorldRead)->OpResult<Vec<Fingerprint>> {
kingmaker_core::diagnostics::measure("guard.world", kingmaker_core::diagnostics::Visibility::Boundary, || {
    Ok(vec![
        fingerprint(&w.provinces())?,
        fingerprint(&w.pops())?,
        fingerprint(&w.enterprises())?,
        fingerprint(&w.stockpiles())?,
        fingerprint(&w.accounts())?,
        fingerprint(&w.resource_operations())?,
        fingerprint(&w.factories())?,
        fingerprint(&w.artisans())?,
        fingerprint(&w.contracts())?,
        fingerprint(&w.expansions())?,
        fingerprint(&w.construction_projects())?,
        fingerprint(&w.completed_construction())?,
        fingerprint(&w.storage_facilities())?,
        fingerprint(&w.storage_reservations())?,
        fingerprint(&w.goods_reservations())?,
        fingerprint(&w.money_reservations())?,
        fingerprint(&w.obligations())?,
        fingerprint(&w.all_committed_operations())?,
        fingerprint(&w.territory())?,
        fingerprint(&w.maintenance_agreements())?,
        fingerprint(&w.sale_offers())?,
        fingerprint(&w.employment())?,
        fingerprint(&w.adventurer_income_rates())?,
        fingerprint(&w.stock_contributions())?,
        fingerprint(&w.institutions())?,
        fingerprint(&w.market_construction_catalog())?,
        fingerprint(&w.jobs())?,
        fingerprint(&w.profiles())?,
        fingerprint(&w.species())?,
        fingerprint(&w.capitalist_entities())?,
        fingerprint(&w.project_investments())?,
        fingerprint(&w.production_beneficiaries())?,
        fingerprint(&w.factory_beneficiaries())?,
        fingerprint(&w.shipments())?,
        fingerprint(&w.transport_assets())?,
        fingerprint(&w.transport_terminals())?,
        fingerprint(&w.supply_routes())?,
        fingerprint(&w.route_reservations())?,
        fingerprint(&w.navigation_geography())?,
        fingerprint(&w.shipment_recovery_plans())?,
        fingerprint(&w.producer_income())?,
        fingerprint(&w.investment_opportunities())?,
        fingerprint(&w.construction_construction_catalog())?,
        fingerprint(&w.building_site_grants())?,
        fingerprint(&w.all_production_phase_receipts())?,
        fingerprint(&w.all_market_phase_receipts())?,
        fingerprint(&w.all_budget_phase_receipts())?,
        fingerprint(&w.all_budget_opening_receipts())?,
        fingerprint(&w.all_labor_phase_receipts())?,
        fingerprint(&w.all_transport_phase_receipts())?,
        fingerprint(&w.all_income_phase_receipts())?,
        fingerprint(&w.all_construction_phase_receipts())?,
        fingerprint(&w.all_week_phase_receipts())?,
        fingerprint(&w.all_mobility_history_receipts())?,
        fingerprint(&w.all_mobility_opening_receipts())?,
        fingerprint(&w.all_mobility_migration_receipts())?,
        fingerprint(&w.all_mobility_profession_receipts())?,
        fingerprint(&w.all_mobility_cohort_receipts())?,
        fingerprint(&w.campaign())?,
        fingerprint(&w.production_policies())?,
        fingerprint(&w.production_catalog())?,
        fingerprint(&w.needs())?,
        fingerprint(&w.price_calibration())?,
        fingerprint(&w.market_parameters())?,
        fingerprint(&w.consumer_facility_by_province())?,
        fingerprint(&w.budget_profile_id())?,
        fingerprint(&w.foundation_budget_policy())?,
        fingerprint(&w.institution_rules())?,
        fingerprint(&w.pop_mobility())?,
        fingerprint(&w.mobility_reference())?,
        fingerprint(&w.automatic_land_paths())?,
        fingerprint(&w.construction_catalog_by_currency())?,

])

})
}


#[cfg(test)]
mod remediation_tests {
    use super::*;
    #[test]
    fn remediation_guard_checks_are_batched_and_hash_identical() {
        use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};
        let rows:Vec<_>=(0..10000).map(|i|serde_json::json!({"id":i,"owner":"owner","quantity":123,"note":"abcdefghijk"})).collect();
        let plain=fingerprint(&rows).unwrap();let count=Arc::new(AtomicUsize::new(0));let seen=count.clone();
        let ctl=crate::run_control::RunControl::new(move |label|{if label=="read_guard"{seen.fetch_add(1,Ordering::Relaxed);}Ok(())});
        assert_eq!(plain,ctl.run(||fingerprint(&rows)).unwrap());
        assert!(count.load(Ordering::Relaxed)<=128,"{} checks for small hash",count.load(Ordering::Relaxed));
    }
    #[test]
    fn remediation_stop_inside_guard_aborts_without_a_digest() {
        let count=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let seen=count.clone();
        let ctl=crate::run_control::RunControl::new(move |label|if label=="read_guard" && seen.fetch_add(1,std::sync::atomic::Ordering::Relaxed)>=3 {Err(OperationError("stop after partial hash".into()))}else{Ok(())});
        assert!(ctl.run(||fingerprint(&vec![0u64;100000])).is_err());assert!(ctl.is_cancelled());assert!(count.load(std::sync::atomic::Ordering::Relaxed)>=4);
    }
}
