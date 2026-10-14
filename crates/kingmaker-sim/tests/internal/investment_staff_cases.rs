#[path="investment_staff_reference.rs"] mod reference;
use kingmaker_core::{world_read::WorldRead,WorldState,construction::ConstructionRequest};
use serde_json::json;
fn snapshot(w:&dyn WorldRead)->serde_json::Value {json!({"raw":w.resource_operations(),"employment":w.employment(),"projects":w.construction_projects(),"operations":w.committed_operations(),"accounts":w.accounts(),"pops":w.pops(),"campaign":w.campaign()})}
fn fixture()->WorldState {
 let mut w=crate::production_invariants_fixture::world();w.production_data.factories.clear();
 w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"raw","province_id":1,"owner_id":"o","method_id":"rgo:iron","resource_id":"iron","assignments":[{"pop_id":"p","employment_id":"e","contract_id":"c","workers_count":1,"time_fraction":1}]})).unwrap());
 w.budget_data.employment.push(serde_json::from_value(json!({"employment_id":"e","contract_id":"c","pop_id":"p","workers_count":1,"time_fraction":1,"fte":1})).unwrap());
 w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"build","status":"funded","assignments":[],"planned_assignments":[{"pop_id":"p","employment_id":"new","contract_id":"c","workers_count":1,"time_fraction":1}],"staff_transfers":[{"source_rgo_id":"raw","employment_id":"e","pop_id":"p","expected_workers_count":1,"workers_count":1}]})).unwrap());w
}
#[test]
fn six_staff_matches_reference_for_partial_stale_groups_and_fallbacks() {
 for case in 0..11 {
  let mut w=fixture();
  match case {
   1=>{w.production_data.construction_projects[0].extra["staff_transfers"][0]["expected_workers_count"]=json!(2);},
   2|3=>{w.production_data.construction_projects[0].extra["planned_assignments"][0]["workers_count"]=json!(2);if case==3 {let mut next=fixture().production_data.construction_projects.remove(0);next.id="next".into();w.production_data.construction_projects.push(next);}},
   4=>{let mut next=fixture().production_data.construction_projects.remove(0);next.id="next".into();next.extra["staff_transfers"][0]["expected_workers_count"]=json!(0);next.extra["staff_transfers"][0]["workers_count"]=json!(0);w.production_data.construction_projects.push(next);},
   5=>{w.production_data.resource_operations[0].base.assignments[0].worker_group_id=Some("group".into());w.production_data.construction_projects[0].extra["planned_assignments"][0]["worker_group_id"]=json!("group");},
   6|7=>{w.production_data.resource_operations[0].base.assignments.clear();w.production_data.construction_projects[0].extra["staff_transfers"]=json!([]);if case==6 {w.production_data.construction_projects[0].extra["planned_assignments"][0]["employment_id"]=json!("e");}else{w.budget_data.employment[0].extra.insert("direct_output_labor".into(),json!(true));}},
   8=>{let mut e=w.budget_data.employment[0].clone();e.employment_id="bad".into();e.pop_id="missing".into();w.budget_data.employment.push(e);},
   9=>{w.production_data.construction_projects[0].status="awaiting_funding".into();},
   10=>{w.production_data.resource_operations[0].base.assignments[0].workers_count=2.;w.production_data.construction_projects[0].extra["staff_transfers"][0]["expected_workers_count"]=json!(2);w.production_data.construction_projects[0].extra["planned_assignments"]=json!([]);},
   _=>{}
  }
  let r=ConstructionRequest{expected_revision:20,period:"w".into(),policies:Default::default()};
  let mut old=crate::world_transaction::WorldTransaction::new(&w);let mut new=crate::world_transaction::WorldTransaction::new(&w);
  let a=reference::activate_staff(&mut old,&r).map(|v|serde_json::to_value(v).unwrap()).map_err(|e|e.to_string());
  let b=crate::investment_staff::activate_staff(&mut new,&r).map(|v|serde_json::to_value(v).unwrap()).map_err(|e|e.to_string());
  assert_eq!(a,b,"outcome case {case}");assert_eq!(snapshot(&old),snapshot(&new),"state case {case}");
 }
}

#[test]
#[ignore="bounded paired release probe"]
fn six_staff_release_probe() {
 for n in [64,128,256] {
  let mut w=fixture();w.production_data.resource_operations.clear();w.budget_data.employment.clear();w.production_data.construction_projects.clear();
  for i in 0..n {w.production_data.resource_operations.push(serde_json::from_value(json!({"id":format!("r{i}"),"province_id":1,"method_id":"rgo:iron","resource_id":"iron","assignments":[]})).unwrap());w.production_data.construction_projects.push(serde_json::from_value(json!({"id":format!("p{i}"),"status":"funded","planned_assignments":[],"assignments":[]})).unwrap());}
  let r=ConstructionRequest{expected_revision:20,period:"w".into(),policies:Default::default()};
  for old in [true,false] {let mut times=vec![];for _ in 0..3 {let t=std::time::Instant::now();let mut tx=crate::world_transaction::WorldTransaction::new(&w);let out=if old{reference::activate_staff(&mut tx,&r)}else{crate::investment_staff::activate_staff(&mut tx,&r)}.unwrap();assert_eq!(out.0.len(),n);times.push(t.elapsed().as_secs_f64());}times.sort_by(f64::total_cmp);eprintln!("staff_release N={n} reference={old} median_ms={}",times[1]*1000.);}
 }
}
