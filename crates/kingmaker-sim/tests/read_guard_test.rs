use kingmaker_core::{world_read::{BorrowedWorld, WorldRead}, *};
use kingmaker_sim::world_transaction::{WorldTransaction, apply_world_delta};
fn stock(id: &str) -> StockpileRecord { StockpileRecord { external_id:id.into(), quantity:3.0, ..Default::default() } }
#[test]
fn changed_value_absent_key_and_phantom_insert_reject_without_overwriting_external_change() {
    for case in 0..3 {
        let mut w = WorldState::default();
        w.stockpiles.push(stock("old"));
        let view = BorrowedWorld::new(&w).unwrap();
        let tx = WorldTransaction::new(&view);
        match case {
            0 => { assert_eq!(tx.stockpiles().get(&"old".into()).unwrap().quantity, 3.0); },
            1 => { assert!(tx.stockpiles().get(&"new".into()).is_none()); },
            _ => { assert_eq!(tx.stockpiles().iter().filter(|r| r.quantity > 0.0).count(), 1); },
        }
        let delta = tx.finish().unwrap();
        drop(view);
        if case == 0 { w.stockpiles[0].quantity = 8.0; } else { w.stockpiles.push(stock("new")); }
        let changed = w.clone();
        assert!(apply_world_delta(&mut w, delta).is_err());
        assert_eq!(w, changed);
    }
}
#[test]
fn row_identity_mismatch_fails_before_any_publication() {
    let mut w = WorldState::default();
    let view = BorrowedWorld::new(&w).unwrap();
    let mut tx = WorldTransaction::new(&view);
    tx.put_stockpiles("key".into(), stock("wrong")).unwrap();
    let delta = tx.finish().unwrap();
    drop(view);
    assert!(apply_world_delta(&mut w, delta).is_err());
    assert!(w.stockpiles.is_empty());
}
