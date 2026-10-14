mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::operations::*;
use kingmaker_sim::operation_session::*;
use serde_json::json;

#[test]
fn food_creation_does_not_invent_a_facility_method_or_currency() {
    let input = json!({"owner_id":"seller","province_id":2,"goods_id":"flour","unit":"t","inventory_origin":"food_conversion","conversion_source_stock_id":"grain:2"});
    let creation = serde_json::from_value::<StockCreate>(input.clone());
    assert!(
        creation.is_ok(),
        "physical consumed food may have no warehouse or industrial method: {creation:?}"
    );
    assert_eq!(serde_json::to_value(creation.unwrap()).unwrap(), input);
}

#[test]
fn purchased_creation_does_not_invent_a_production_method() {
    let input = json!({"owner_id":"a","province_id":1,"goods_id":"grain","unit":"t","facility_id":"wh:a","currency_id":"COIN","inventory_origin":"purchased"});
    let creation = serde_json::from_value::<StockCreate>(input.clone());
    assert!(
        creation.is_ok(),
        "a purchased stock is not a factory activity: {creation:?}"
    );
    assert_eq!(serde_json::to_value(creation.unwrap()).unwrap(), input);
}

#[test]
fn direct_purchase_and_conversion_cannot_change_goods_or_money() {
    for kind in ["purchase", "food_conversion"] {
        let world = market_world();
        let before = world.clone();
        let mut s = OperationSession::new(&world).unwrap();
        let operation = serde_json::from_value::<EconomicOperation>(
            json!({"event_id":"untrusted","type":kind,"debits":[],"credits":[],"stock_changes":[{"stock_id":"grain:2","delta":-1}],"quantity":1,"goods_id":"grain"}),
        );
        assert!(
            operation.is_ok(),
            "typed operation must exist before testing its trust boundary"
        );
        assert!(s.stage(&operation.unwrap()).is_err());
        assert!(s.finish().is_err());
        assert_eq!(world, before);
    }
}
