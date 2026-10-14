#[path = "../common/market.rs"]
mod market_fixture;
use crate::{market_clearing::*, operation_session::*};
use kingmaker_core::market::*;
use kingmaker_core::*;
use market_fixture::*;
use serde_json::json;
fn request() -> MarketRequest {
    MarketRequest {
        expected_revision: 20,
        period: "w".into(),
        stage: MarketStage::ClosingMarket,
        profile: Default::default(),
        priorities: None,
    }
}
#[test]
fn partial_destination_storage_with_paid_rent_and_release() {
    let mut w = market_world();
    w.accounts[0].purpose = "employer_operating_capital".into();
    w.production_data.storage_facilities[0].capacity = Some(3.0);
    w.production_data.storage_facilities[0].owner_id = "landlord".into();
    w.production_data.storage_facilities[0].access_contracts=serde_json::from_value(json!([{"owner_id":"a","status":"source","source":"lease","price_minor_per_space":"10","currency_id":"COIN","purpose":"employer_operating_capital","cancellation_policy":"refund_unused"}])).unwrap();
    w.stockpiles[0].quantity = 2.0;
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request()).unwrap();
    let r = plan_foundation_market(
        &mut s,
        "w",
        &[order(
            "input",
            "a",
            "grain",
            10.0,
            "production_inputs",
            false,
        )],
        &["production_inputs".into()],
    )
    .unwrap();
    assert_eq!(r.fulfilled.get("input"), Some(&2.0));
    let row = s
        .storage_reservation(&"market-storage:w:input".into())
        .unwrap();
    assert_eq!(row.status, "released");
    assert_eq!(row.consumed_quantity, Some(2.0));
    assert_eq!(row.released_quantity, Some(1.0));
    assert_eq!(
        s.cash(&"a".into(), &"COIN".into(), "employer_operating_capital"),
        Money(9780)
    );
    let changes = s.finish().unwrap();
    apply_prepared(&mut w, changes).unwrap();
    assert_eq!(w.stockpiles[0].province_id, ProvinceId(2));
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.owner_id.as_str() == Some("a"))
            .unwrap()
            .quantity,
        2.0
    );
}
#[test]
fn personal_beneficiary_held_and_changed_sale_rights() {
    let mut w = market_world();
    w.stockpiles[0].owner_id = "b".into();
    w.stockpiles[0].purpose = "food_reserve".into();
    w.stockpiles[0].beneficiary_pop_id = Some("b".into());
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request()).unwrap();
    let r = plan_foundation_market(
        &mut s,
        "w",
        &[order("a", "a", "grain", 1.0, "basic", true)],
        &["basic".into()],
    )
    .unwrap();
    assert!(r.fulfilled.is_empty());
    assert!(s.stock(&"grain:2".into()).unwrap().quantity == 20.0);
    w.stockpiles[0].purpose = "commercial".into();
    w.stockpiles[0].beneficiary_pop_id = None;
    w.market_data.sale_offers=serde_json::from_value(json!([{"stock_id":"grain:2","seller_id":"b","goods_id":"grain","seller_purpose":"personal_savings","status":"approved","source":"authored","approval_ref":"approval"}])).unwrap();
    w.production_data.goods_reservations =
        serde_json::from_value(json!([{"stock_id":"grain:2","status":"held","quantity":19}]))
            .unwrap();
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request()).unwrap();
    let r = plan_foundation_market(
        &mut s,
        "w",
        &[order("a", "a", "grain", 10.0, "basic", true)],
        &["basic".into()],
    )
    .unwrap();
    assert_eq!(r.fulfilled.get("a"), Some(&1.0));
    assert_eq!(
        s.cash(&"b".into(), &"COIN".into(), "personal_savings"),
        Money(10100)
    );
    w.market_data.territory[0].region_id = Some(7.into());
    w.market_data.territory[1].region_id = Some("7".into());
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request()).unwrap();
    let r = plan_foundation_market(
        &mut s,
        "w",
        &[order("a", "a", "grain", 1.0, "basic", true)],
        &["basic".into()],
    )
    .unwrap();
    assert!(r.fulfilled.is_empty());
}
#[test]
fn foundation_stored_operations_match_actual_js() {
    let mut w = market_world();
    w.accounts[0].purpose = "employer_operating_capital".into();
    w.production_data.storage_facilities[0].capacity = Some(3.0);
    let orders = vec![order(
        "input",
        "a",
        "grain",
        10.0,
        "production_inputs",
        false,
    )];
    let priorities = vec!["production_inputs".into()];
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request()).unwrap();
    let r = plan_foundation_market(&mut s, "w", &orders, &priorities).unwrap();
    assert_eq!(r.fulfilled.get("input"), Some(&3.0));
    let js = oracle(
        json!({"action":"foundation","snapshot":snapshot(&w),"period":"w","orders":orders,"priorities":priorities}),
    );
    let expected: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(js["operations"].clone()).unwrap();
    assert_source_values(
        &serde_json::to_value(r.operations).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
}

#[test]
fn artisan_input_and_project_materials_stay_private() {
    let mut w = market_world();
    w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"artisan:flour","kind":"artisan","output_goods_id":"flour","output_unit":"t","status":"ready","weekly_productivity":1,"inputs":[{"goods_id":"grain","amount":1}],"staffing":["artisans"]})).unwrap());
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"artisan","owner_id":"seller","province_id":2,"method_id":"artisan:flour","working_capital":{"currency_id":"COIN"},"assignments":[{"pop_id":"b","workers_count":2,"time_fraction":1}]})).unwrap());
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request()).unwrap();
    let r = plan_foundation_market(
        &mut s,
        "w",
        &[order("a", "a", "grain", 20.0, "basic", true)],
        &["basic".into()],
    )
    .unwrap();
    assert!(r.fulfilled.is_empty());
    w.market_data.sale_offers=serde_json::from_value(json!([{"stock_id":"grain:2","seller_id":"seller","goods_id":"grain","seller_purpose":"employer_operating_capital","status":"source","source":"authored"}])).unwrap();
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request()).unwrap();
    let r = plan_foundation_market(
        &mut s,
        "w",
        &[order("a", "a", "grain", 20.0, "basic", true)],
        &["basic".into()],
    )
    .unwrap();
    assert_eq!(r.fulfilled.get("a"), Some(&18.0));
    w.production_data.artisans.clear();
    w.market_data.sale_offers.clear();
    w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"p","status":"building","owner_id":"seller","province_id":2,"construction":{"materials":[{"goods_id":"grain","amount":2}]},"assignments":[]})).unwrap());
    let mut s = OperationSession::new(&w).unwrap();
    s.configure_market(&request()).unwrap();
    let r = plan_foundation_market(
        &mut s,
        "w",
        &[order("a", "a", "grain", 20.0, "basic", true)],
        &["basic".into()],
    )
    .unwrap();
    assert!(r.fulfilled.is_empty());
}
