#[path="common/native_week.rs"] mod common;
use kingmaker_core::{diagnostics::{ProfileSession}, *};
use kingmaker_sim::native_week::*;
use std::{cell::RefCell,rc::Rc,io::{self,Write}};
#[derive(Clone,Default)]struct Capture(Rc<RefCell<Vec<u8>>>);impl Write for Capture{fn write(&mut self,b:&[u8])->io::Result<usize>{self.0.borrow_mut().extend_from_slice(b);Ok(b.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
#[test]fn profile_preserves_week_and_replay(){
 let mut a=common::factory(false);let mut b=a.clone();let request=common::request(&a);
 let plain=execute_native_week(&mut a,&request,|_|{}).unwrap();let c=Capture::default();let profile=ProfileSession::install(Box::new(c.clone())).unwrap();
 let measured=execute_native_week(&mut b,&request,|_|{}).unwrap();let before=serde_json::to_value(&b).unwrap();
 assert_eq!(execute_native_week(&mut b,&request,|_|panic!("replay executed")).unwrap(),measured);assert_eq!(serde_json::to_value(&b).unwrap(),before);
 profile.finish().unwrap();
 assert_eq!(a.accounts,b.accounts);assert_eq!(a.stockpiles,b.stockpiles);assert_eq!(a.pops,b.pops);assert_eq!(a.campaign,b.campaign);
 assert_eq!(a.production_data.storage_reservations,b.production_data.storage_reservations);assert_eq!(a.production_data.obligations,b.production_data.obligations);
 assert_eq!(plain.operation_evidence,measured.operation_evidence);assert_eq!(plain.registry_evidence,measured.registry_evidence);
 let text=String::from_utf8(c.0.borrow().clone()).unwrap();
 for name in ["world.index","week.prepare","week.history","phase.production","guard.world","week.apply"]{assert!(text.contains(name),"missing {name}");}
}
#[test]fn profile_preserves_cancellation_and_stale_refusal(){
 for profiled in [false,true]{
  let session=profiled.then(||ProfileSession::install(Box::new(io::sink())).unwrap());
  let mut w=common::factory(false);let request=common::request(&w);let before=serde_json::to_value(&w).unwrap();
  assert!(try_prepare_native_week(&w,&request,|phase|if phase=="income"{Err(kingmaker_sim::operation_session::OperationError("CANCELLED_BY_OPERATOR".into()))}else{Ok(())}).is_err());
  assert_eq!(serde_json::to_value(&w).unwrap(),before);
  let prepared=prepare_native_week(&w,&request,|_|{}).unwrap();w.accounts[0].balance.0+=1;let changed=serde_json::to_value(&w).unwrap();assert!(apply_native_week(&mut w,prepared).is_err());assert_eq!(serde_json::to_value(&w).unwrap(),changed);
  if let Some(s)=session{s.finish().unwrap();}
 }
}
#[test]fn invalid_request_closes_prepare_with_error(){
 let w=common::empty();let mut r=common::request(&w);r.expected_revision+=1;
 let c=Capture::default();let s=ProfileSession::install(Box::new(c.clone())).unwrap();assert!(prepare_native_week(&w,&r,|_|{}).is_err());s.finish().unwrap();
 let rows:Vec<serde_json::Value>=String::from_utf8(c.0.borrow().clone()).unwrap().lines().map(|s|serde_json::from_str(s).unwrap()).collect();
 let id=&rows.iter().find(|v|v["name"]=="week.prepare").expect("prepare scope")["id"];
 assert!(rows.iter().any(|v|v["event"]=="end"&&v["id"]==*id&&v["outcome"]=="error"));
}
