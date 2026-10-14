#[path = "common/production.rs"]
mod production_fixture;
mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::*;
use kingmaker_sim::{market_inputs::*, production::*};
use serde_json::json;
#[test]
fn zero_actual_output_still_orders_potential_recipe_inputs() {
    let mut w = production_fixture::world();
    w.stockpiles.clear();
    let receipt = execute_production_cycle(&mut w, &production_fixture::request()).unwrap();
    assert_eq!(receipt.outputs[0].quantity, 0.0);
    let rows = build_foundation_input_orders(&w, &receipt).unwrap();
    assert_eq!(rows.len(), 2);
    assert_close(rows[0].quantity, 1.1, 1e-9);
    assert_close(rows[1].quantity, 0.4, 1e-9);
    let mut forged = receipt.clone();
    forged.procurement_activities[0].potential_output = Some(1000.0);
    assert!(build_foundation_input_orders(&w, &forged).is_err());
}
#[test]
fn pending_retooling_and_owned_held_inputs_match_js() {
    let mut w = production_fixture::world();
    w.stockpiles[0].quantity = 0.3;
    w.production_data.goods_reservations =
        serde_json::from_value(json!([{"stock_id":"iron","quantity":0.2,"status":"held"}]))
            .unwrap();
    let receipt = execute_production_cycle(&mut w, &production_fixture::request()).unwrap();
    let rows = build_foundation_input_orders(&w, &receipt).unwrap();
    let js = oracle(
        json!({"action":"inputs","snapshot":snapshot(&w),"activities":receipt.procurement_activities}),
    );
    let expected: Vec<kingmaker_core::market::MarketOrder> = serde_json::from_value(js).unwrap();
    assert_source_values(
        &serde_json::to_value(rows).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
    let mut pending = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    pending.method_id = "factory:new".into();
    pending.weekly_productivity = Some(2.0);
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .push(pending);
    w.production_data.factories[0].base.extra.insert(
        "pending_retooling".into(),
        json!({"method_id":"factory:new"}),
    );
    let rows = build_foundation_input_orders(&w, &receipt).unwrap();
    assert!(
        rows.iter()
            .find(|o| o.goods_id.as_str() == Some("iron"))
            .unwrap()
            .quantity
            > 2.0
    );
    let expected:Vec<kingmaker_core::market::MarketOrder>=serde_json::from_value(oracle(json!({"action":"inputs","snapshot":snapshot(&w),"activities":receipt.procurement_activities}))).unwrap();
    assert_source_values(
        &serde_json::to_value(rows).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
}
#[test]
fn construction_status_limits_and_escrow() {
    let mut w = production_fixture::world();
    let receipt = execute_production_cycle(&mut w, &production_fixture::request()).unwrap();
    w.production_data.construction_projects=serde_json::from_value(json!([{"id":"new","status":"funded","kind":"school","owner_id":"builder","province_id":1,"construction":{"status":"ready","materials":[{"goods_id":"iron","amount":5}]},"terms":{"currency_id":"COIN"},"material_facility_ids":{"iron":"wh:builder"}},{"id":"cancelled","status":"cancelled","kind":"school","owner_id":"builder","province_id":1,"construction":{"status":"ready","materials":[{"goods_id":"iron","amount":500}]},"terms":{"currency_id":"COIN"}}])).unwrap();
    let orders = build_foundation_input_orders(&w, &receipt).unwrap();
    let row = orders
        .iter()
        .find(|o| o.id == "construction-input:new:iron")
        .unwrap();
    assert_eq!(row.quantity, 5.0);
    assert_eq!(row.purpose, "construction_escrow");
    assert!(!orders.iter().any(|o| o.id.contains("cancelled")));
    w.market_data.construction_catalog = vec![json!({"id":"school","unique":true})];
    w.production_data.completed_construction.push(
        serde_json::from_value(
            json!({"id":"built","province_id":1,"kind":"school","status":"completed","level":1}),
        )
        .unwrap(),
    );
    assert!(!build_foundation_input_orders(&w, &receipt)
        .unwrap()
        .iter()
        .any(|o| o.id.starts_with("construction-input")));
}
#[test]
fn land_path_uses_real_land_edges_and_same_nonempty_landmass() {
    let mut w = market_world();
    assert_eq!(
        find_land_path(&w.market_data, ProvinceId(1), ProvinceId(3)).unwrap(),
        Some(vec![ProvinceId(1), ProvinceId(2), ProvinceId(3)])
    );
    w.market_data.territory[1].extra.insert(
        "neighbors".into(),
        json!([{"neighbor_id":3,"contact_type":"sea"}]),
    );
    assert_eq!(
        find_land_path(&w.market_data, ProvinceId(1), ProvinceId(3)).unwrap(),
        None
    );
    w.market_data.territory[2]
        .extra
        .insert("landmass_id".into(), json!("other"));
    assert_eq!(
        find_land_path(&w.market_data, ProvinceId(3), ProvinceId(1)).unwrap(),
        None
    );
}
