#[path = "common/construction.rs"]
mod common;
use kingmaker_core::*;
use serde_json::json;
fn ready(kind: &str) -> WorldState {
    let mut w = common::ready(7);
    let p = &mut w.production_data.construction_projects[0];
    p.extra.insert("kind".into(), json!(kind));
    p.extra.get_mut("terms").unwrap()["operating_reserve_minor"] = json!("0");
    p.extra.get_mut("construction").unwrap()["effects"] = json!({"authored_effect":3});
    if kind == "mine" {
        p.extra.insert("target_rgo_id".into(), json!("raw"));
        w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"raw","owner_id":"raw-owner","province_id":1,"method_id":"rgo:iron","resource_id":"iron","resource_status":"approved","land_access":true,"assignments":[]})).unwrap());
    }
    w
}
#[test]
fn mine_completion_adds_capacity_by_existing_rgo_rules() {
    let mut w = ready("mine");
    kingmaker_sim::construction_cycle::execute_construction(&mut w, &common::request()).unwrap();
    assert_eq!(w.production_data.expansions.len(), 1);
    assert_eq!(
        kingmaker_sim::production_activities::rgo_capacity(
            &w.production_data.resource_operations[0],
            &w.production_data.expansions
        )
        .unwrap(),
        42000.
    );
    assert_eq!(w.production_data.expansions[0].places_per_level, 2000.);
    assert_eq!(common::cash(&w, "a", "personal_savings"), 2700);
    assert_eq!(common::cash(&w, "b", "personal_savings"), 6300);
}
#[test]
fn ordinary_building_keeps_authored_effects_and_asset_rights() {
    let mut w = ready("school");
    kingmaker_sim::construction_cycle::execute_construction(&mut w, &common::request()).unwrap();
    assert_eq!(w.production_data.completed_construction.len(), 1);
    let b = serde_json::to_value(&w.production_data.completed_construction[0]).unwrap();
    assert_eq!(b["effects"], json!({"authored_effect":3}));
    assert_eq!(b["asset"]["shares"][0]["numerator"], json!("3"));
}
#[test]
fn illegal_mine_resource_and_unique_building_wait_without_spending() {
    for case in 0..2 {
        let mut w = ready(if case == 0 { "mine" } else { "school" });
        if case == 0 {
            w.production_data.resource_operations[0].resource_id = "salt".into();
        } else {
            w.construction_data.construction_catalog = vec![json!({"id":"school","unique":true})];
            w.production_data.completed_construction.push(
                serde_json::from_value(
                    json!({"id":"existing","province_id":1,"kind":"school","status":"completed"}),
                )
                .unwrap(),
            );
        }
        let stocks = w.stockpiles.clone();
        let accounts = w.accounts.clone();
        let receipt =
            kingmaker_sim::construction_cycle::execute_construction(&mut w, &common::request())
                .unwrap();
        assert!(!receipt.diagnostics.is_empty());
        assert_eq!(w.stockpiles, stocks);
        assert_eq!(w.accounts, accounts);
    }
}
#[test]
fn completed_building_records_refunds_and_uses_net_investor_capital() {
    let mut w = common::ready_with(7, |w| {
        w.accounts[2].balance = Money(1);
        w.accounts[3].balance = Money(2);
        let opportunity = &mut w.construction_data.investment_opportunities[0];
        opportunity["terms"]["budget_minor"] = json!("3");
        opportunity["terms"]["operating_reserve_minor"] = json!("0");
        w.production_data.catalog.as_mut().unwrap().methods[0]
            .extra
            .get_mut("construction")
            .unwrap()["cost_minor"] = json!("1");
        w.production_data.contracts[0].weekly_wage_rate =
            Some(serde_json::from_value(json!({"numerator":"1","denominator":"1"})).unwrap());
    });
    w.production_data.construction_projects[0]
        .extra
        .insert("kind".into(), json!("school"));
    kingmaker_sim::construction_cycle::execute_construction(&mut w, &common::request()).unwrap();
    let p = serde_json::to_value(&w.production_data.construction_projects[0]).unwrap();
    assert_eq!(p["status"], json!("completed"));
    assert_eq!(p["refunded_minor"], json!("2"));
    let asset = serde_json::to_value(&w.production_data.completed_construction[0]).unwrap()
        ["asset"]
        .clone();
    assert_eq!(
        asset["shares"],
        json!([{"investor_id":"b","owner_id":"b","numerator":"1","denominator":"1"}])
    );
    assert_eq!(asset["capital_value_minor"], json!("1"));
}
