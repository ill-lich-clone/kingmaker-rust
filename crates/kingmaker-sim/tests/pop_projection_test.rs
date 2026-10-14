#[path="../src/pop_invariant_projection.rs"] mod projection;
#[path="common/allocation_meter.rs"] mod meter;
#[global_allocator]static ALLOC:meter::Allocator=meter::Allocator;
use kingmaker_core::{world_read::WorldRead, *};
use serde_json::json;
#[test]
fn remediation_every_pop_field_except_account_binding_is_protected() {
 let mut before=WorldState::default();before.pops.push(PopRecord::default());
 let changes=json!({"extra":{"authored":1},"state_id":"x","id":7,"external_id":"x","province_id":7,"pop_type":"x","pop_type_id":"x","culture_id":"x","religion_id":"x","size":7,"workforce":7.0,"employed":7.0,"unemployed":7.0,"currency_id":"x","maintenance_payer_id":"x","culture":"x","religion":"x","literacy":0.5,"militancy":0.5,"consciousness":0.5});
 let base=serde_json::to_value(&before.pops[0]).unwrap();
 for (field,value) in changes.as_object().unwrap() {
  let mut altered=base.clone();altered[field]=value.clone();let mut after=before.clone();after.pops[0]=serde_json::from_value(altered).unwrap();
  assert!(!projection::same_people_except_accounts(before.pops(),after.pops()),"accepted changed {field}");
 }
 assert_eq!(base.as_object().unwrap().len(),changes.as_object().unwrap().len()+2,"new field needs explicit test");
 let mut after=before.clone();after.pops[0].account_id=Some(AccountId(9));after.pops[0].account_status="bound".into();
 assert!(projection::same_people_except_accounts(before.pops(),after.pops()));
 after.pops.push(PopRecord::default());assert!(!projection::same_people_except_accounts(before.pops(),after.pops()));
}
#[test]
fn remediation_pop_projection_has_constant_temporary_allocation() {
 let _lock=meter::MEASURE.lock().unwrap();
 for n in [1000,2000,4000] {
  let mut w=WorldState::default();w.pops=(0..n).map(|i|PopRecord{external_id:format!("p{i}").into(),pop_type:"craftsmen".into(),..Default::default()}).collect();
  let measurement=meter::Measurement::start();let equal=projection::same_people_except_accounts(w.pops(),w.pops());let (total,peak)=measurement.finish();
  assert!(equal);assert!(total<64*n);assert!(peak<64*1024,"peak={peak}");
 }
}
