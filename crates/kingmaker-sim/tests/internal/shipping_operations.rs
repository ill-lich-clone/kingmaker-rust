#[path = "../common/production.rs"]
mod common;
use crate::{operation_session::*, shipping_operations::stage_shipping_operations};
use kingmaker_core::{operations::*, *};
use serde_json::json;
fn dispatch() -> EconomicOperation {
    serde_json::from_value(json!({"event_id":"native-dispatch","type":"shipment_dispatch","shipment_id":"cargo","debits":[{"owner_id":"o","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"10"}],"credits":[{"owner_id":"shipment:cargo","currency_id":"COIN","purpose":"trade_escrow","amount_minor":"6"},{"owner_id":"carrier","currency_id":"COIN","purpose":"operating_capital","amount_minor":"4"}],"stock_changes":[{"stock_id":"iron","delta":-0.5}]})).unwrap()
}
#[test]
fn private_shipping_dispatch_and_arrival_conserve_cash_and_allow_exact_stock_creation() {
    let mut world = common::world();
    let money = world.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let mut session = OperationSession::new(&world).unwrap();
    stage_shipping_operations(&mut session, &[dispatch()]).unwrap();
    let changes = session.finish().unwrap();
    apply_prepared(&mut world, changes).unwrap();
    assert!((world.stockpiles[0].quantity - 0.6).abs() < 1e-10);
    world.mobility_data.shipments.push(json!({"id":"cargo","buyer_id":"buyer","seller_id":"seller","goods_id":"iron","remaining_quantity":0.5,"shipped_quantity":0.5}));
    let arrival:EconomicOperation=serde_json::from_value(json!({"event_id":"native-arrival","type":"shipment_arrival","shipment_id":"cargo","debits":[{"owner_id":"shipment:cargo","currency_id":"COIN","purpose":"trade_escrow","amount_minor":"6"}],"credits":[{"owner_id":"seller","currency_id":"COIN","purpose":"operating_capital","amount_minor":"6"}],"stock_changes":[{"stock_id":"delivered","delta":0.5,"create":{"owner_id":"buyer","province_id":1,"goods_id":"iron","unit":"t","currency_id":"COIN","inventory_origin":"purchased"}}]})).unwrap();
    let mut session = OperationSession::new(&world).unwrap();
    stage_shipping_operations(&mut session, &[arrival]).unwrap();
    let changes = session.finish().unwrap();
    apply_prepared(&mut world, changes).unwrap();
    assert_eq!(
        world.accounts.iter().map(|a| a.balance.0).sum::<i64>(),
        money
    );
    assert_eq!(
        world.stockpiles.last().unwrap().owner_id,
        ExternalId::from("buyer")
    );
    assert_eq!(world.stockpiles.last().unwrap().quantity, 0.5);
}
#[test]
fn private_shipping_late_failure_poison_and_wrong_stock_direction_reject() {
    let world = common::world();
    let first = dispatch();
    let mut bad = first.clone();
    bad.event_id = "bad".into();
    bad.credits[0].amount_minor = Money(7);
    let mut session = OperationSession::new(&world).unwrap();
    assert!(stage_shipping_operations(&mut session, &[first, bad]).is_err());
    assert!(session.finish().is_err());
    for (kind, delta) in [
        (OperationKind::ShipmentDispatch, 0.5),
        (OperationKind::ShipmentArrival, -0.5),
    ] {
        let mut bad = dispatch();
        bad.kind = kind;
        bad.stock_changes[0].delta = delta;
        let mut session = OperationSession::new(&world).unwrap();
        assert!(stage_shipping_operations(&mut session, &[bad]).is_err());
        assert!(session.finish().is_err());
    }
}
#[test]
fn shipping_arrival_requires_real_cargo_and_preserves_actual_producer_parts() {
    for case in 0..4 {
        let mut w = common::world();
        w.production_data.artisans.push(serde_json::from_value(json!({"id":"artisan","owner_id":"o","province_id":1,"method_id":"artisan:iron","assignments":[]})).unwrap());
        if case != 1 {
            w.mobility_data.shipments.push(json!({"id":"cargo","buyer_id":"buyer","goods_id":"iron","remaining_quantity":if case==2{0.1}else{0.5},"producer_contributions":[{"producer_id":"artisan","quantity":if case==3{0.8}else{0.25}}]}));
        }
        let arrival:EconomicOperation=serde_json::from_value(json!({"event_id":"arrival","type":"shipment_arrival","shipment_id":"cargo","stock_changes":[{"stock_id":"new","delta":0.5,"create":{"owner_id":"buyer","province_id":1,"goods_id":"iron","unit":"t","inventory_origin":"purchased"}}]})).unwrap();
        let mut session = OperationSession::new(&w).unwrap();
        stage_shipping_operations(&mut session, &[arrival]).unwrap();
        let result = session.finish();
        if case == 0 {
            apply_prepared(&mut w, result.unwrap()).unwrap();
            assert_eq!(w.income_data.stock_contributions.len(), 1);
            assert_eq!(
                w.income_data.stock_contributions[0].producer_id,
                ExternalId::from("artisan")
            );
            assert_eq!(w.income_data.stock_contributions[0].quantity, 0.25);
        } else {
            let before=serde_json::to_value(&w).unwrap();
            assert!(apply_prepared(&mut w,result.unwrap()).is_err(),"case {case}");
            assert_eq!(serde_json::to_value(&w).unwrap(),before);
        }
    }
}
