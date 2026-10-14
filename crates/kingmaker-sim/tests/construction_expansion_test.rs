#[path = "common/construction.rs"]
mod common;
use kingmaker_core::*;
use serde_json::json;
fn ready() -> WorldState {
    let mut w = common::ready(7);
    let project = &mut w.production_data.construction_projects[0];
    project
        .extra
        .insert("target_factory_id".into(), json!("old"));
    let shares = json!([{"investor_id":"a","owner_id":"a","numerator":"1","denominator":"1"}]);
    project.extra.insert(
        "expansion_base".into(),
        json!({"level":1,"capital_value_minor":"5000","shares":shares}),
    );
    project
        .extra
        .insert("existing_owner_acceptances".into(), json!(["a"]));
    w.production_data.factories.push(serde_json::from_value(json!({"id":"old","owner_id":"old-owner","enterprise_id":"old-owner","province_id":1,"region_id":"r","method_id":"factory:steel","facility_id":"old-wh","level":1,"capital_value_minor":"5000","shares":shares,"status":"awaiting_staff","people_slots":40000,"equipment_capacity_weekly":100,"equipment_source":"fixture","assignments":[],"parameters":{"source":"fixture","status":"source","tenure_Bmax":0,"tenure_Tsat":30}})).unwrap());
    let mut facility = w.production_data.storage_facilities[0].clone();
    facility.id = "old-wh".into();
    facility.owner_id = "old-owner".into();
    facility.capacity = Some(100.);
    w.production_data.storage_facilities.push(facility);
    w.refresh_enterprise_views().unwrap();
    w
}
#[test]
fn expansion_keeps_existing_capital_and_combines_net_rights_and_capacity() {
    let mut w = ready();
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let result =
        kingmaker_sim::construction_cycle::execute_construction(&mut w, &common::request())
            .unwrap();
    assert!(result.invariants.all_required_passed());
    assert_eq!(w.production_data.factories.len(), 1);
    let f = serde_json::to_value(&w.production_data.factories[0]).unwrap();
    assert_eq!(f["id"], json!("old"));
    assert_eq!(f["owner_id"], json!("old-owner"));
    assert_eq!(f["level"], json!(2));
    assert_eq!(f["people_slots"].as_f64(), Some(80000.));
    assert_eq!(f["equipment_capacity_weekly"].as_f64(), Some(200.));
    assert_eq!(f["capital_value_minor"], json!("10000"));
    assert_eq!(
        f["shares"],
        json!([{"investor_id":"a","owner_id":"a","numerator":"13","denominator":"20"},{"investor_id":"b","owner_id":"b","numerator":"7","denominator":"20"}])
    );
    assert_eq!(w.production_data.storage_facilities[1].capacity, Some(200.));
    assert_eq!(
        common::cash(&w, "old-owner", "employer_operating_capital"),
        4000
    );
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
}
#[test]
fn missing_consent_or_changed_original_rights_rolls_back_all_progress() {
    for case in 0..3 {
        let mut w = ready();
        match case {
            0 => {
                w.production_data.construction_projects[0]
                    .extra
                    .insert("existing_owner_acceptances".into(), json!([]));
            }
            1 => {
                w.production_data.factories[0]
                    .base
                    .extra
                    .insert("capital_value_minor".into(), json!("6000"));
            }
            _ => {
                w.production_data.factories[0]
                    .base
                    .extra
                    .insert("shares".into(), json!([]));
            }
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(kingmaker_sim::construction_cycle::execute_construction(
            &mut w,
            &common::request()
        )
        .is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn expansion_at_level_twenty_waits_without_consuming_materials() {
    let mut w = ready();
    w.production_data.factories[0]
        .base
        .extra
        .insert("level".into(), json!(20));
    let stocks = w.stockpiles.clone();
    let result =
        kingmaker_sim::construction_cycle::execute_construction(&mut w, &common::request())
            .unwrap();
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d["reason"] == "factory_expansion_limit"));
    assert_eq!(w.stockpiles, stocks);
    assert_ne!(
        w.production_data.construction_projects[0].status,
        "completed"
    );
}
