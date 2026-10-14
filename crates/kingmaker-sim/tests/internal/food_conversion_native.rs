#[path = "../common/market.rs"]
mod market_fixture;
use crate::{food_conversion::*, operation_session::*};
use kingmaker_core::market::*;
use kingmaker_core::*;
use market_fixture::*;
use serde_json::json;
fn request(stage: MarketStage) -> MarketRequest {
    MarketRequest {
        expected_revision: 20,
        period: "w".into(),
        stage,
        profile: Default::default(),
        priorities: None,
    }
}
#[test]
fn grain_and_livestock_exact_actual_ration_and_no_unused_virtual_party() {
    for (raw, food, coefficient) in [("grain", "flour", 1.03), ("livestock", "meat", 5.0)] {
        let mut w = market_world();
        w.stockpiles[0].goods = Goods::from_external(raw.into());
        w.stockpiles[0].quantity = coefficient * 10.0;
        let mut s = OperationSession::new(&w).unwrap();
        s.configure_market(&request(MarketStage::EarlyFood))
            .unwrap();
        let mut o = order("food", "a", food, 1.0, "basic", true);
        o.food_alternatives = vec![json!({"goods_id":food,"share":1})];
        let r = plan_food_conversion(&mut s, "w", &[o.clone()], FoodStage::Early).unwrap();
        assert_eq!(r.converted.len(), 1);
        assert_close(
            s.stock(&"grain:2".into()).unwrap().quantity,
            coefficient * 9.0,
            1e-9,
        );
        assert_eq!(
            r.receipts
                .iter()
                .map(|o| o.meta["quantity"].as_f64().unwrap())
                .sum::<f64>(),
            1.0
        );
        assert!(s
            .stocks()
            .filter(|x| x.goods.external_id().as_str() == Some(food))
            .all(|x| x.quantity == 0.0));
        let js = oracle(
            json!({"action":"food","snapshot":snapshot(&w),"period":"w","orders":[o],"stage":"early"}),
        );
        let expected: Vec<kingmaker_core::operations::EconomicOperation> =
            serde_json::from_value(js["operations"].clone()).unwrap();
        assert_source_values(
            &serde_json::to_value(r.operations).unwrap(),
            &serde_json::to_value(expected).unwrap(),
        );
    }
}
#[test]
fn ready_food_first_unpaid_hunger_and_approved_pop_conversion_right() {
    let mut w = market_world();
    let mut ready = w.stockpiles[0].clone();
    ready.id = 2;
    ready.external_id = "ready".into();
    ready.goods = Goods::from_external("flour".into());
    ready.quantity = 1.0;
    w.stockpiles.push(ready);
    let o = order("food", "a", "flour", 1.0, "basic", true);
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::EarlyFood))
        .unwrap();
    let r = plan_food_conversion(&mut s, "w", &[o.clone()], FoodStage::Early).unwrap();
    assert!(r.converted.is_empty());
    assert_eq!(s.stock(&"grain:2".into()).unwrap().quantity, 20.0);
    w.stockpiles.remove(1);
    w.accounts[0].balance = Money(0);
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::EarlyFood))
        .unwrap();
    let r = plan_food_conversion(&mut s, "w", &[o.clone()], FoodStage::Early).unwrap();
    assert!(r.converted.is_empty());
    assert_eq!(s.stock(&"grain:2".into()).unwrap().quantity, 20.0);
    w.accounts[0].balance = Money(1000);
    w.stockpiles[0].owner_id = "b".into();
    w.market_data.sale_offers=serde_json::from_value(json!([{"id":"offer","stock_id":"grain:2","seller_id":"b","goods_id":"grain","seller_purpose":"personal_savings","status":"source","source":"author"}])).unwrap();
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::EarlyFood))
        .unwrap();
    let r = plan_food_conversion(&mut s, "w", &[o.clone()], FoodStage::Early).unwrap();
    assert_eq!(r.converted.len(), 1);
    assert_eq!(
        s.cash(&"b".into(), &"COIN".into(), "personal_savings"),
        Money(10100)
    );
    let js = oracle(
        json!({"action":"food","snapshot":snapshot(&w),"period":"w","orders":[o],"stage":"early"}),
    );
    let expected: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(js["operations"].clone()).unwrap();
    assert_source_values(
        &serde_json::to_value(r.operations).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
}
#[test]
fn grain_flour_alternatives_merge_and_remaining_quota() {
    let mut o = order("food", "a", "grain", 10.0, "basic", true);
    o.food_alternatives = vec![
        json!({"goods_id":"grain","share":0.3,"H":3}),
        json!({"goods_id":"flour","share":0.7,"H":7}),
    ];
    o.food_gross_quantity = Some(10.0);
    let rows = food_orders(&[o.clone()]).unwrap();
    assert_eq!(rows[0].food_alternatives.len(), 1);
    assert_eq!(rows[0].food_alternatives[0]["share"], json!(1.0));
    assert_eq!(rows[0].food_alternatives[0]["H"], json!(10.0));
    let receipt=serde_json::from_value(json!({"event_id":"e","type":"purchase","order_id":"fallback:a:food:fish","pop_id":"a","quantity":4,"need_id":"food","consumed":true})).unwrap();
    let rows = remaining_food_orders(&[o], &[receipt]).unwrap();
    assert_eq!(rows[0].quantity, 6.0);
    assert_eq!(rows[0].food_gross_quantity, Some(6.0));
}
#[test]
fn surplus_respects_funded_raw_demand_and_net_space() {
    let mut w = market_world();
    w.stockpiles[0].quantity = 10.3;
    w.production_data.storage_facilities[2].capacity = Some(10.3);
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::ClosingMarket))
        .unwrap();
    let r = plan_food_conversion(&mut s, "w", &[], FoodStage::Surplus).unwrap();
    assert_eq!(r.converted.len(), 1);
    assert_close(s.stock(&"grain:2".into()).unwrap().quantity, 0.0, 1e-9);
    assert_close(
        s.stocks()
            .filter(|x| x.goods.external_id().as_str() == Some("flour"))
            .map(|x| x.quantity)
            .sum(),
        10.0,
        1e-9,
    );
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::ClosingMarket))
        .unwrap();
    let mut raw = order("input", "a", "grain", 10.3, "production_inputs", false);
    raw.purpose = "personal_savings".into(); // this fixture funds only the personal wallet
    let r = plan_food_conversion(&mut s, "w", &[raw], FoodStage::Surplus).unwrap();
    assert!(r.converted.is_empty());
    assert!(r
        .blocked
        .iter()
        .any(|b| b["reason"] == "other_funded_raw_demand"));
}

#[test]
fn partial_military_and_nonfood_maintenance_match_js() {
    let mut w = market_world();
    let mut supply = w.stockpiles[0].clone();
    supply.id = 2;
    supply.external_id = "military".into();
    supply.goods = Goods::from_external("military_supply".into());
    supply.quantity = 2.5;
    w.stockpiles.push(supply);
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("military_supply".into(), json!(1));
    w.production_data.catalog.as_mut().unwrap().goods.push(
        serde_json::from_value(
            json!({"goods_id":"military_supply","kind":"processed_good","unit":"t"}),
        )
        .unwrap(),
    );
    let mut food = order("food", "a", "grain", 5.0, "basic", true);
    food.food_gross_quantity = Some(10.0);
    food.military_food_substitution = Some(5.0);
    food.food_alternatives = vec![json!({"goods_id":"grain","share":1})];
    let mut military = order("military", "a", "military_supply", 5.0, "basic", true);
    military.need_id = Some("military_supply".into());
    let mut clothing = order("clothing", "a", "clothing", 1.0, "basic", true);
    clothing.need_id = Some("clothing".into());
    clothing.maintenance = true;
    let orders = vec![food, military, clothing];
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::EarlyFood))
        .unwrap();
    let r = plan_food_conversion(&mut s, "w", &orders, FoodStage::Early).unwrap();
    assert_close(
        r.receipts
            .iter()
            .filter(|o| o.meta.get("need_id") == Some(&json!("food")))
            .map(|o| o.meta["quantity"].as_f64().unwrap())
            .sum(),
        7.5,
        1e-9,
    );
    let js = oracle(
        json!({"action":"food","snapshot":snapshot(&w),"period":"w","orders":orders,"stage":"early"}),
    );
    let expected: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(js["operations"].clone()).unwrap();
    assert_source_values(
        &serde_json::to_value(r.operations).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
}

// Both stocks initially occupy 20.6 space; 4.85 free space permits only
// 5 tonnes of flour because its net space cost is 2 - 1.03 = 0.97 per tonne.
fn shared_surplus_warehouse(rented: bool) -> WorldState {
    let mut w = market_world();
    w.stockpiles[0].quantity = 10.3;
    let mut second = w.stockpiles[0].clone();
    second.id = 2;
    second.external_id = "second-grain".into();
    w.stockpiles[0].owner_id = "outsider".into();
    w.stockpiles.push(second);
    let f = &mut w.production_data.storage_facilities[2];
    f.capacity = Some(25.45);
    f.goods.get_mut("flour").unwrap().space_per_unit = 2.;
    if rented {
        f.access_contracts.push(serde_json::from_value(json!({
            "owner_id":"outsider","status":"source","source":"author",
            "price_minor_per_space":"1000","currency_id":"COIN",
            "purpose":"operating_capital","cancellation_policy":"refund_unused"
        })).unwrap());
        w.accounts.push(AccountBalance { id:AccountId(4), external_id:"outsider-cash".into(),
            owner_id:"outsider".into(), currency_id:"COIN".into(),
            purpose:"operating_capital".into(), balance:Money(100) });
        w.rebuild_account_index().unwrap();
    }
    w
}

#[test]
fn rejected_surplus_lease_does_not_give_next_stock_fictitious_free_space() {
    let w = shared_surplus_warehouse(false);
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::ClosingMarket)).unwrap();
    let r = plan_food_conversion(&mut s,"w",&[],FoodStage::Surplus).unwrap();
    assert_eq!(r.converted.len(),1);
    assert!(r.blocked.iter().any(|b| b["reason"]=="storage_access_denied"));
    assert_close(s.stock(&"grain:2".into()).unwrap().quantity,10.3,1e-9);
    assert_close(s.stock(&"second-grain".into()).unwrap().quantity,5.15,1e-9);
    let flour:f64=s.stocks().filter(|s|s.goods.external_id().as_str()==Some("flour")).map(|s|s.quantity).sum();
    assert_close(flour,5.,1e-9);
}

#[test]
fn partial_surplus_rent_releases_only_the_raw_quantity_actually_converted() {
    let w = shared_surplus_warehouse(true);
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::ClosingMarket)).unwrap();
    let r = plan_food_conversion(&mut s,"w",&[],FoodStage::Surplus).unwrap();
    assert_eq!(r.converted.len(),2);
    assert_close(s.stock(&"grain:2".into()).unwrap().quantity,10.2485,1e-9);
    assert_close(s.stock(&"second-grain".into()).unwrap().quantity,5.2015,1e-9);
    let flour:f64=s.stocks().filter(|s|s.goods.external_id().as_str()==Some("flour")).map(|s|s.quantity).sum();
    assert_close(flour,5.,1e-9);
    assert_eq!(s.cash(&"outsider".into(),&"COIN".into(),"operating_capital"),Money(0));
    assert_eq!(s.cash(&"seller".into(),&"COIN".into(),"operating_capital"),Money(100));
}
#[test]
fn conversion_failure_poison_rolls_back_ready_purchase_and_rights_are_retained() {
    let mut w = market_world();
    let mut ready = w.stockpiles[0].clone();
    ready.id = 2;
    ready.external_id = "ready".into();
    ready.goods = Goods::from_external("flour".into());
    ready.quantity = 0.5;
    w.stockpiles.push(ready);
    w.stockpiles[0].unit = Some(json!("wrong-unit"));
    let before = w.clone();
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::EarlyFood))
        .unwrap();
    let mut o = order("food", "a", "flour", 1.0, "basic", true);
    o.food_alternatives = vec![json!({"goods_id":"flour","share":1})];
    assert!(plan_food_conversion(&mut s, "w", &[o.clone()], FoodStage::Early).is_err());
    assert!(s.finish().is_err());
    assert_eq!(w, before);
    w.stockpiles.remove(1);
    w.stockpiles[0].unit = Some(json!("t"));
    w.stockpiles[0].owner_id = "b".into();
    w.stockpiles[0].purpose = "food_reserve".into();
    w.stockpiles[0].beneficiary_pop_id = Some("b".into());
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::EarlyFood))
        .unwrap();
    let r = plan_food_conversion(&mut s, "w", &[o], FoodStage::Early).unwrap();
    assert!(r.converted.is_empty());
}
#[test]
fn surplus_paid_rent_uses_real_net_space_and_source_operations() {
    let mut w = market_world();
    w.stockpiles[0].quantity = 10.3;
    w.production_data.storage_facilities[2].capacity = Some(10.3);
    w.production_data.storage_facilities[2].owner_id = "landlord".into();
    w.production_data.storage_facilities[2].access_contracts=serde_json::from_value(json!([{"owner_id":"seller","status":"source","source":"lease","price_minor_per_space":"10","currency_id":"COIN","purpose":"employer_operating_capital","cancellation_policy":"refund_unused"}])).unwrap();
    w.accounts[2].balance = Money(50);
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request(MarketStage::ClosingMarket))
        .unwrap();
    let r = plan_food_conversion(&mut s, "w", &[], FoodStage::Surplus).unwrap();
    assert_close(
        r.converted[0]["output_quantity"].as_f64().unwrap(),
        5.0,
        1e-9,
    );
    assert_eq!(
        s.cash(
            &"seller".into(),
            &"COIN".into(),
            "employer_operating_capital"
        ),
        Money(0)
    );
    let js = oracle(
        json!({"action":"food","snapshot":snapshot(&w),"period":"w","orders":[],"stage":"surplus"}),
    );
    let mut expected: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(js["operations"].clone()).unwrap();
    let mut actual = r.operations;
    expected.sort_by_key(|o| serde_json::to_string(&o.event_id).unwrap());
    actual.sort_by_key(|o| serde_json::to_string(&o.event_id).unwrap());
    assert_source_values(
        &serde_json::to_value(actual).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
}
