#[path="common/native_week.rs"] mod common;
#[path="common/allocation_meter.rs"] mod meter;
#[global_allocator] static ALLOC:meter::Allocator=meter::Allocator;
use kingmaker_core::operations::CommittedOperation;
use kingmaker_sim::native_week::execute_native_week;
fn run(payload:usize)->usize {
    let mut w=common::empty();
    for i in 0..8u32 {w.production_data.committed_operations.push(CommittedOperation {event_id:format!("old:{i}").into(),value:serde_json::json!({"event_id":format!("old:{i}"),"type":"blocked","notes":"x".repeat(payload)})});}
    let r=common::request(&w);
    let m=meter::Measurement::start();
    let receipt=execute_native_week(&mut w,&r,|_|{}).unwrap();
    let (bytes,_)=m.finish();
    assert_eq!(w.campaign.revision,r.expected_revision+1);
    assert!(receipt.invariants.production_required_passed());
    bytes
}
#[test]
fn whole_week_has_one_overlay_and_no_deep_world_snapshots() {
    let _lock=meter::MEASURE.lock().unwrap();let small=run(0);let large=run(16384);
    assert!(large.saturating_sub(small)<512*1024,"week copied old payloads: small={small} large={large}");
}

