#[path = "common/production.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::production::*;
use serde_json::json;
use std::collections::HashMap;
#[test]
fn real_rgo_produces_100_grain_and_50_iron_into_owned_stocks() {
    let mut w = common::world();
    w.production_data.factories.clear();
    w.stockpiles.clear();
    w.pops[0].workforce = 100.0;
    w.pops[0].size = 100;
    w.pops[0].pop_type_id = "farmers".into();
    let mut p = w.pops[0].clone();
    p.id = PopId(2);
    p.external_id = "p2".into();
    p.province_id = ProvinceId(2);
    w.pops.push(p);
    w.provinces.push(ProvinceRecord {
        id: ProvinceId(2),
        ..Default::default()
    });
    w.production_data.contracts[0]
        .weekly_wage_rate
        .as_mut()
        .unwrap()
        .numerator = "0".into();
    w.production_data.contracts[0].guaranteed_idle_weekly_rate = None;
    w.production_data.storage_facilities.clear();
    for (province, pop, good, rate, employment) in
        [(1, "p", "grain", 1.0, "e"), (2, "p2", "iron", 0.5, "e2")]
    {
        let id = format!("rgo:{good}");
        let facility = format!("warehouse:{province}");
        w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":id,"kind":"rgo","status":"ready","output_goods_id":good,"output_unit":"t","weekly_productivity":rate,"inputs":[],"staffing":["farmers"]})).unwrap());
        w.production_data.resource_operations.push(serde_json::from_value(json!({"id":id,"owner_id":"o","province_id":province,"method_id":id,"resource_id":good,"resource_status":"approved","approval_ref":"map","land_access":true,"facility_id":facility,"assignments":[{"pop_id":pop,"workers_count":100,"time_fraction":1,"contract_id":"c","employment_id":employment}]})).unwrap());
        w.production_data.storage_facilities.push(serde_json::from_value(json!({"id":facility,"owner_id":"o","province_id":province,"capacity":1000,"unit":"slots","status":"source","source":"author","goods":{good:{"unit":"t","space_per_unit":1}}})).unwrap());
    }
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .goods
        .push(serde_json::from_value(json!({"goods_id":"grain","kind":"raw_resource"})).unwrap());
    let receipt = execute_production_cycle(&mut w, &common::request()).unwrap();
    assert_eq!(
        receipt
            .outputs
            .iter()
            .find(|r| r.goods_id.as_str() == Some("grain"))
            .unwrap()
            .quantity,
        100.0
    );
    assert_eq!(
        receipt
            .outputs
            .iter()
            .find(|r| r.goods_id.as_str() == Some("iron"))
            .unwrap()
            .quantity,
        50.0
    );
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.goods == Goods::Grain)
            .unwrap()
            .quantity,
        100.0
    );
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.goods == Goods::Iron)
            .unwrap()
            .quantity,
        50.0
    );
}
#[test]
fn detached_legacy_production_refuses_before_mutation() {
    let mut stocks = vec![StockpileRecord::default()];
    let before = stocks.clone();
    assert!(execute_world_production(&[], &[], &mut stocks, &HashMap::new()).is_err());
    assert_eq!(stocks, before);
}
