use kingmaker_core::{world_read::{BorrowedWorld, WorldRead}, *};
use kingmaker_sim::world_transaction::{WorldTransaction, apply_world_delta};
#[path = "common/allocation_meter.rs"] mod meter;
#[global_allocator] static ALLOC: meter::Allocator = meter::Allocator;
#[test]
fn nested_rollback_restores_rows_ids_and_deletions_without_copying_other_payloads() {
    let _lock = meter::MEASURE.lock().unwrap();
    let mut w = WorldState::default();
    w.stockpiles = (0..10000).map(|n| StockpileRecord { id:n+1, external_id:n.into(), quantity:10.0, extra:serde_json::from_value(serde_json::json!({"notes":"x".repeat(1024)})).unwrap(), ..Default::default() }).collect();
    let view = BorrowedWorld::new(&w).unwrap();
    let m = meter::Measurement::start();
    let mut tx = WorldTransaction::new(&view);
    let mut row = tx.stockpiles().get(&0.into()).unwrap().clone();
    row.quantity = 4.0;
    tx.put_stockpiles(0.into(), row).unwrap();
    let savepoint = tx.savepoint();
    let id = tx.allocate_id().unwrap();
    tx.remove_stockpiles(0.into()).unwrap();
    tx.put_stockpiles("new".into(), StockpileRecord { external_id:"new".into(), ..Default::default() }).unwrap();
    tx.put_obligations("debt".into(), serde_json::from_value(serde_json::json!({"obligation_id":"debt","payer_id":"a","payee_id":"b","currency_id":"c","purpose":"wages","amount_minor":5})).unwrap()).unwrap();
    assert!(tx.stockpiles().get(&0.into()).is_none());
    assert_eq!(tx.stockpiles().iter().count(), 10000);
    tx.rollback_to(savepoint).unwrap();
    assert_eq!(tx.allocate_id().unwrap(), id);
    assert_eq!(tx.stockpiles().get(&0.into()).unwrap().quantity, 4.0);
    assert!(tx.stockpiles().get(&"new".into()).is_none());
    assert!(tx.obligations().get(&"debt".into()).is_none());
    let delta = tx.finish().unwrap();
    let (allocated, _) = m.finish();
    // Iteration hashes stream into the digest; no Value/world serialization buffer.
    assert!(allocated < 2 * 1024 * 1024, "one changed row allocated {allocated}");
    assert_eq!(delta.view(&view).stockpiles().get(&0.into()).unwrap().quantity, 4.0);
    drop(view);
    apply_world_delta(&mut w, delta).unwrap();
    assert_eq!(w.stockpiles.len(), 10000);
    assert_eq!(w.stockpiles[0].quantity, 4.0);
    assert_eq!(w.stockpiles[1].quantity, 10.0);
}

#[test]
fn foreign_savepoint_cannot_rollback_another_transaction() {
    let w = WorldState::default();
    let view = BorrowedWorld::new(&w).unwrap();
    let mut a = WorldTransaction::new(&view);
    let mut b = WorldTransaction::new(&view);
    assert!(b.rollback_to(a.savepoint()).is_err());
}

#[test]
fn failed_nested_phase_restores_money_reservations_events_and_campaign() {
    let mut w = WorldState::default();
    let account = AccountBalance { owner_id:"a".into(), currency_id:"c".into(), purpose:"cash".into(), balance:Money(20), ..Default::default() };
    w.accounts.push(account.clone());
    let view = BorrowedWorld::new(&w).unwrap();
    let mut tx = WorldTransaction::new(&view);
    let first = tx.savepoint();
    let mut paid = account.clone(); paid.balance=Money(3);
    tx.put_accounts(paid.address(),paid).unwrap();
    tx.put_storage_reservations("r".into(),serde_json::from_value(serde_json::json!({"id":"r","owner_id":"a","facility_id":"wh","goods_id":"grain","unit":"t","status":"held","quantity":3})).unwrap()).unwrap();
    tx.put_committed_operations("e".into(),kingmaker_core::operations::CommittedOperation {event_id:"e".into(),value:serde_json::json!({"paid":17})}).unwrap();
    let mut campaign=tx.campaign().clone(); campaign.revision=7;
    tx.set_campaign(campaign).unwrap();
    let stale=tx.savepoint();
    tx.poison();
    tx.rollback_to(first).unwrap();
    assert!(tx.rollback_to(stale).is_err());
    assert_eq!(tx.accounts().get(&account.address()).unwrap().balance,Money(20));
    assert!(tx.storage_reservations().get(&"r".into()).is_none());
    assert!(tx.committed_operations().get(&"e".into()).is_none());
    assert_eq!(tx.campaign().revision,0);
    tx.finish().unwrap();
    let mut tx=WorldTransaction::new(&view);
    tx.poison();
    assert!(tx.finish().is_err());
}

#[test]
fn ordered_replacement_and_later_creation_preserve_explicit_order_and_rollback() {
    let mut w=WorldState::default();
    for n in 1..=3 {w.pops.push(PopRecord {id:PopId(n),external_id:n.into(),..Default::default()});}
    let mut tx=WorldTransaction::new(&w);
    let mark=tx.savepoint();
    tx.replace_pops(vec![w.pops[2].clone(),w.pops[0].clone()]).unwrap();
    tx.put_pops(4.into(),PopRecord {id:PopId(4),external_id:4.into(),..Default::default()}).unwrap();
    assert_eq!(tx.pops().iter().map(|p|p.id.0).collect::<Vec<_>>(),[3,1,4]);
    tx.rollback_to(mark).unwrap();
    assert_eq!(tx.pops().iter().map(|p|p.id.0).collect::<Vec<_>>(),[1,2,3]);
    tx.replace_pops(vec![w.pops[2].clone(),w.pops[0].clone()]).unwrap();
    let d=tx.finish().unwrap();
    apply_world_delta(&mut w,d).unwrap();
    assert_eq!(w.pops.iter().map(|p|p.id.0).collect::<Vec<_>>(),[3,1]);
}

#[test]
fn remediation_overlay_positions_after_mutation() {
    let row=|id:&str,q:f64| StockpileRecord{external_id:id.into(),quantity:q,..Default::default()};
    let mut w=WorldState::default();w.stockpiles=vec![row("a",1.),row("b",2.)];
    let base=BorrowedWorld::new(&w).unwrap();let mut tx=WorldTransaction::new(&base);
    let assert_rows=|tx:&dyn WorldRead,expected:&[(&str,f64)]|{
        let rows=tx.stockpiles(); assert_eq!(rows.len(),expected.len());
        for(i,(id,q))in expected.iter().enumerate(){let r=rows.at(i).unwrap();assert_eq!(r.external_id,ExternalId::from(*id));assert_eq!(r.quantity,*q);}
        assert!(rows.at(expected.len()).is_none());
    };
    assert_rows(&tx,&[("a",1.),("b",2.)]);let mark=tx.savepoint();
    tx.remove_stockpiles("a".into()).unwrap();assert_rows(&tx,&[("b",2.)]);
    tx.put_stockpiles("c".into(),row("c",3.)).unwrap();assert_rows(&tx,&[("b",2.),("c",3.)]);
    tx.put_stockpiles("a".into(),row("a",4.)).unwrap();assert_rows(&tx,&[("a",4.),("b",2.),("c",3.)]);
    tx.replace_stockpiles(vec![row("c",3.),row("b",5.),row("a",4.)]).unwrap();assert_rows(&tx,&[("c",3.),("b",5.),("a",4.)]);
    let mut child=WorldTransaction::new(&tx);child.remove_stockpiles("b".into()).unwrap();let delta=child.finish().unwrap();
    assert_rows(&delta.view(&tx),&[("c",3.),("a",4.)]);tx.apply_delta(delta).unwrap();assert_rows(&tx,&[("c",3.),("a",4.)]);
    tx.rollback_to(mark).unwrap();assert_rows(&tx,&[("a",1.),("b",2.)]);
    let delta=tx.finish().unwrap();drop(base);
    w.stockpiles.push(row("phantom",6.));let before=w.clone();assert!(apply_world_delta(&mut w,delta).is_err());assert_eq!(w,before);
}
