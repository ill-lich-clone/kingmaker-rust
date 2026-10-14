#[path="common/labor.rs"] mod common;
#[path="common/allocation_meter.rs"] mod meter;
#[global_allocator] static ALLOC:meter::Allocator=meter::Allocator;
use kingmaker_core::{labor::LaborRequest,operations::CommittedOperation};
use kingmaker_sim::labor_cycle::execute_local_labor;
fn run(payload:usize)->usize {
    let mut w=common::world(1540);
    for i in 0..64u32 {
        w.production_data.committed_operations.push(CommittedOperation {event_id:format!("old:{i}").into(),value:serde_json::json!({"event_id":format!("old:{i}"),"type":"blocked","notes":"x".repeat(payload)})});
    }
    let r=LaborRequest {expected_revision:w.campaign.revision,period:"w".into(),policies:w.production_data.policies.clone()};
    let measurement=meter::Measurement::start();
    let receipt=execute_local_labor(&mut w,&r).unwrap();
    let (bytes,_)=measurement.finish();
    assert_eq!(receipt.hired_fte,300.);
    assert_eq!(w.pops[0].employed,300.);
    bytes
}
#[test]
fn labor_does_not_copy_unrelated_history_payloads() {
    let _lock=meter::MEASURE.lock().unwrap();
    let small=run(0);let large=run(32768);
    assert!(large.saturating_sub(small)<1024*1024,"labor copied unrelated history: small={small}, large={large}");
}

