#![allow(dead_code)]
#[path = "market.rs"]
mod market;
#[path = "production.rs"]
mod production;
use kingmaker_core::*;
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};
pub fn world(cash: i64) -> WorldState {
    let mut w = market::market_world();
    w.pops[0].pop_type_id = "farmers".into();
    w.pops[0]
        .extra
        .insert("species_labor_coefficient".into(), json!(1));
    w.market_data.needs.as_mut().unwrap().profiles[0].norms.push(serde_json::from_value(json!({"need_id":"clothing","goods_id":"clothing","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":0.002,"unit":"t","allocation_rule":"none"})).unwrap());
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("clothing".into(), json!(3));
    w.production_data.catalog=Some(serde_json::from_value(json!({"version":"v","goods":[{"goods_id":"grain","kind":"raw_resource","unit":"t"}],"methods":[{"method_id":"rgo:grain","kind":"rgo","status":"ready","output_goods_id":"grain","output_unit":"t","weekly_productivity":1,"staffing":["farmers"],"inputs":[]}]})).unwrap());
    w.production_data.resource_operations=serde_json::from_value(json!([{"id":"rgo","owner_id":"o","province_id":1,"method_id":"rgo:grain","facility_id":"wh","resource_id":"grain","resource_status":"approved","approval_ref":"fixture","land_access":true,"assignments":[]}])).unwrap();
    w.production_data.factories.clear();
    w.production_data.artisans.clear();
    w.production_data.contracts.clear();
    w.budget_data.employment.clear();
    w.budget_data.jobs.clear();
    w.accounts
        .push(production::account(101, "o", "operating_capital", cash));
    w.rebuild_account_index().unwrap();
    w
}
pub fn snapshot(w: &WorldState) -> Value {
    json!({"registry":{"pops":w.pops.iter().map(|p|{let mut v=json!({"pop_id":p.external_id,"state_id":p.state_id,"province_id":p.province_id.0,"pop_type_id":p.pop_type_id,"size":p.size,"workforce":p.workforce,"currency_id":p.currency_id});v.as_object_mut().unwrap().extend(p.extra.clone());v}).collect::<Vec<_>>(),"profiles":w.labor_data.profiles,"jobs":w.budget_data.jobs,"employment":w.budget_data.employment,"contracts":w.production_data.contracts,"resourceOperations":w.production_data.resource_operations,"factoryEnterprises":w.production_data.factories,"artisanEnterprises":w.production_data.artisans,"productionCatalog":w.production_data.catalog,"storageFacilities":w.production_data.storage_facilities,"goodsReservations":w.production_data.goods_reservations,"needs":w.market_data.needs,"priceCalibration":w.market_data.price_calibration,"policies":w.production_data.policies,"catalogs":{"species":w.labor_data.species}},"accounts":w.accounts.iter().map(|a|json!({"owner_id":a.owner_id,"currency_id":a.currency_id,"purpose":a.purpose,"amount_minor":a.balance.0.to_string()})).collect::<Vec<_>>(),"stocks":w.stockpiles.iter().map(|s|json!({"opening_stock_id":s.external_id,"owner_id":s.owner_id,"province_id":s.province_id.0,"goods_id":s.goods.external_id(),"amount":s.quantity,"unit":s.unit,"facility_id":s.facility_id})).collect::<Vec<_>>(),"obligations":w.production_data.obligations,"reservations":w.production_data.storage_reservations})
}
pub fn oracle(input: Value) -> Value {
    let mut c = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/labor-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = c.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
