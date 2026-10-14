#[path = "common/production.rs"]
mod common;
use kingmaker_core::storage::*;
use kingmaker_core::*;
use kingmaker_sim::operation_session::*;
use kingmaker_sim::storage::*;
use serde_json::json;
#[test]
fn storage_respects_rights_units_and_held_space() {
    let mut w = common::world();
    w.production_data.storage_facilities[0].capacity = Some(10.0);
    let mut s = OperationSession::new(&w).unwrap();
    let mut batch = StorageBatch::new(&s).unwrap();
    let req = StorageRequest {
        id: "r".into(),
        owner_id: "o".into(),
        facility_id: "wh".into(),
        goods_id: "steel".into(),
        unit: json!("t"),
        quantity: 6.0,
    };
    let r = reserve_storage(&mut s, &mut batch, &req, "w").unwrap();
    assert_eq!(r.reservation.unwrap().quantity, 6.0);
    let mut req2 = req.clone();
    req2.id = "r2".into();
    req2.quantity = 6.0;
    assert_eq!(
        reserve_storage(&mut s, &mut batch, &req2, "w")
            .unwrap()
            .reservation
            .unwrap()
            .quantity,
        4.0
    );
    let mut bad = req.clone();
    bad.id = "bad".into();
    bad.owner_id = "outsider".into();
    assert_eq!(
        reserve_storage(&mut s, &mut batch, &bad, "w")
            .unwrap()
            .reason
            .as_deref(),
        Some("storage_access_denied")
    );
    bad.owner_id = "o".into();
    bad.unit = json!("pieces");
    assert_eq!(
        reserve_storage(&mut s, &mut batch, &bad, "w")
            .unwrap()
            .reason
            .as_deref(),
        Some("storage_unit_mismatch")
    );
    assert_eq!(
        reserve_storage(&mut s, &mut batch, &req, "w")
            .unwrap()
            .reservation
            .unwrap()
            .quantity,
        6.0
    );
    let changes = s.finish().unwrap();
    apply_prepared(&mut w, changes).unwrap();
    assert_eq!(w.production_data.storage_reservations.len(), 2);
}
#[test]
fn partial_production_refunds_only_unused_space_and_preserves_debt() {
    let mut w = common::world();
    w.accounts[0].balance = Money(100);
    w.production_data.storage_facilities[0].owner_id = "provider".into();
    w.production_data.storage_facilities[0].access_contracts=vec![serde_json::from_value::<LeaseTerms>(json!({"owner_id":"o","status":"source","source":"law","price_minor_per_space":"10","currency_id":"COIN","purpose":"employer_operating_capital","cancellation_policy":"refund_unused"})).unwrap()];
    let mut s = OperationSession::new(&w).unwrap();
    let mut batch = StorageBatch::new(&s).unwrap();
    let req = StorageRequest {
        id: "r".into(),
        owner_id: "o".into(),
        facility_id: "wh".into(),
        goods_id: "steel".into(),
        unit: json!("t"),
        quantity: 10.0,
    };
    reserve_storage(&mut s, &mut batch, &req, "w").unwrap();
    consume_storage(&mut s, &mut batch, &"r".into(), 4.0).unwrap();
    // Provider spends part of rent before cancellation, through an ordinary legal transfer.
    s.stage(&kingmaker_core::operations::EconomicOperation::transfer(
        "spend".into(),
        AccountAddress {
            owner_id: "provider".into(),
            currency_id: "COIN".into(),
            purpose: "operating_capital".into(),
        },
        AccountAddress {
            owner_id: "o".into(),
            currency_id: "COIN".into(),
            purpose: "operating_capital".into(),
        },
        Money(75),
    ))
    .unwrap();
    release_storage(&mut s, &mut batch, &"r".into(), &"release".into()).unwrap();
    release_storage(&mut s, &mut batch, &"r".into(), &"release".into()).unwrap();
    let row = s.storage_reservation(&"r".into()).unwrap();
    assert_eq!(row.consumed_quantity, Some(4.0));
    assert_eq!(row.released_quantity, Some(6.0));
    assert_eq!(
        s.debt(&"debt:release".into()).unwrap().amount_minor,
        Money(35)
    );
    assert_eq!(
        s.cash(&"o".into(), &"COIN".into(), "employer_operating_capital"),
        Money(25)
    );
    let d = s.finish().unwrap();
    apply_prepared(&mut w, d).unwrap();
    assert_eq!(w.production_data.committed_operations.len(), 3);
}
#[test]
fn unknown_capacity_and_failed_batch_never_publish() {
    let mut w = common::world();
    w.production_data.storage_facilities[0].capacity = None;
    let mut s = OperationSession::new(&w).unwrap();
    let mut b = StorageBatch::new(&s).unwrap();
    let req = StorageRequest {
        id: "r".into(),
        owner_id: "o".into(),
        facility_id: "wh".into(),
        goods_id: "steel".into(),
        unit: json!("t"),
        quantity: 1.0,
    };
    assert_eq!(
        reserve_storage(&mut s, &mut b, &req, "w")
            .unwrap()
            .reason
            .as_deref(),
        Some("storage_parameters_unknown")
    );
    w.production_data.storage_facilities[0].capacity = Some(100.0);
    let mut s = OperationSession::new(&w).unwrap();
    let mut b = StorageBatch::new(&s).unwrap();
    reserve_storage(&mut s, &mut b, &req, "w").unwrap();
    let mut changed = req.clone();
    changed.quantity = 2.0;
    assert!(reserve_storage(&mut s, &mut b, &changed, "w").is_err());
    assert!(s.finish().is_err());
    assert!(w.production_data.storage_reservations.is_empty());
}
