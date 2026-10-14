use kingmaker_core::{operations::*, world_read::WorldRead, *};
use kingmaker_sim::{operation_session::OperationSession, world_transaction::{WorldTransaction,apply_world_delta}};
#[path="common/allocation_meter.rs"] mod meter;
#[global_allocator] static ALLOC:meter::Allocator=meter::Allocator;
#[test]
fn session_borrows_old_history_and_phase_changes_are_visible_before_week_publication() {
    let _lock=meter::MEASURE.lock().unwrap();
    let mut w=WorldState::default();
    let from=AccountAddress {owner_id:"a".into(),currency_id:"c".into(),purpose:"cash".into()};
    let to=AccountAddress {owner_id:"b".into(),..from.clone()};
    w.accounts.push(AccountBalance {id:AccountId(1),external_id:"a-cash".into(),owner_id:from.owner_id.clone(),currency_id:from.currency_id.clone(),purpose:from.purpose.clone(),balance:Money(100)});
    w.accounts.push(AccountBalance {id:AccountId(2),external_id:"b-cash".into(),owner_id:to.owner_id.clone(),currency_id:to.currency_id.clone(),purpose:to.purpose.clone(),balance:Money(0)});
    for n in 0..2000u32 {
        w.production_data.committed_operations.push(CommittedOperation {event_id:n.into(),value:serde_json::json!({"event_id":n,"type":"blocked","notes":"x".repeat(4096)})});
    }
    let view=kingmaker_core::world_read::BorrowedWorld::new(&w).unwrap();
    let mut tx=WorldTransaction::new(&view);
    let m=meter::Measurement::start();
    let mut s=OperationSession::new(&tx).unwrap();
    let (bytes,_)=m.finish();
    assert!(bytes<512*1024,"session copied old history: {bytes}");
    let op=EconomicOperation::transfer("new".into(),from.clone(),to.clone(),Money(7));
    s.stage(&op).unwrap();
    let changes=s.finish().unwrap();
    tx.apply_phase(changes).unwrap();
    assert_eq!(tx.accounts().get(&from).unwrap().balance,Money(93));
    assert_eq!(tx.accounts().get(&to).unwrap().balance,Money(7));
    assert_eq!(w.accounts[0].balance,Money(100));
    let delta=tx.finish().unwrap();
    drop(view);
    apply_world_delta(&mut w,delta).unwrap();
    assert_eq!(w.accounts[0].balance,Money(93));
    assert_eq!(w.production_data.committed_operations.len(),2001);
}

#[test]
fn session_replays_archived_typed_history_without_materializing_the_archive() {
    use kingmaker_core::history::*;
    use std::sync::Arc;
    let mut w=WorldState::default();
    let op=EconomicOperation::transfer("archived".into(),AccountAddress{owner_id:"a".into(),currency_id:"c".into(),purpose:"cash".into()},AccountAddress{owner_id:"b".into(),currency_id:"c".into(),purpose:"cash".into()},Money(7));
    let value=serde_json::to_value(&op).unwrap();
    let mut archive=MemoryHistory::default();
    archive.append(HistoryRecord{key:HistoryKey{namespace:"committed_operations".into(),id:op.event_id.clone()},value:serde_json::to_value(CommittedOperation{event_id:op.event_id.clone(),value:value.clone()}).unwrap()}).unwrap();
    w.history=Some(HistoryBacking::new(Arc::new(archive)));
    let mut session=OperationSession::new(&w).unwrap();
    assert_eq!(session.committed(&op.event_id).unwrap().as_ref(),&value);
    assert_eq!(session.stage(&op).unwrap(),kingmaker_sim::operation_session::ApplyOutcome::AlreadyApplied);
    assert!(w.production_data.committed_operations.is_empty());
}
