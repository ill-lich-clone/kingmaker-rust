#[path = "common/production.rs"]
mod common;
use kingmaker_core::operations::*;
use kingmaker_core::*;
use kingmaker_sim::{operation_session::*, production::*};
use serde_json::json;
fn slave_world() -> WorldState {
    let mut w = common::world();
    w.production_data.factories.clear();
    w.pops[0].pop_type_id = "slaves".into();
    w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"rgo:iron","kind":"rgo","status":"ready","output_goods_id":"iron","output_unit":"t","weekly_productivity":1,"inputs":[],"staffing":["slaves"]})).unwrap());
    w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"r","owner_id":"o","province_id":1,"method_id":"rgo:iron","resource_id":"iron","resource_status":"approved","approval_ref":"map","land_access":true,"facility_id":"wh","assignments":[{"pop_id":"p","workers_count":1,"time_fraction":1}]})).unwrap());
    w.production_data.storage_facilities[0].goods.insert(
        "iron".into(),
        serde_json::from_value(json!({"unit":"t","space_per_unit":1})).unwrap(),
    );
    w
}
#[test]
fn unrelated_old_event_cannot_prove_slave_maintenance() {
    let mut w = slave_world();
    w.production_data
        .committed_operations
        .push(CommittedOperation {
            event_id: "old".into(),
            value: json!({"event_id":"old","type":"blocked","reason":"unrelated"}),
        });
    let mut req = common::request();
    req.maintenance_receipts.push(
        serde_json::from_value(json!({"pop_id":"p","period":"w","fraction":1,"event_ids":["old"]}))
            .unwrap(),
    );
    assert_eq!(
        execute_production_cycle(&mut w, &req).unwrap().outputs[0].quantity,
        0.0
    );
}
#[test]
fn empty_processed_recipe_and_wrong_steel_recipe_reject_atomically() {
    let mut w = common::world();
    w.production_data.factories.clear();
    w.pops[0].pop_type_id = "artisans".into();
    let m = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
    m.kind = "artisan".into();
    m.method_id = "artisan:steel".into();
    m.staffing = vec!["artisans".into()];
    m.inputs.clear();
    w.stockpiles.clear();
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"a","owner_id":"o","province_id":1,"method_id":"artisan:steel","facility_id":"wh","people_slots":1,"assignments":[{"pop_id":"p","workers_count":1,"time_fraction":1}]})).unwrap());
    let before = w.clone();
    assert!(execute_production_cycle(&mut w, &common::request()).is_err());
    assert_eq!(w, before);
    let mut w = common::world();
    w.production_data.catalog.as_mut().unwrap().methods[0].inputs[0].amount = 1.0;
    let before = w.clone();
    assert!(execute_production_cycle(&mut w, &common::request()).is_err());
    assert_eq!(w, before);
}
#[test]
fn kilograms_cannot_satisfy_tonnes_without_conversion() {
    let mut w = common::world();
    w.production_data.catalog.as_mut().unwrap().goods[0]
        .extra
        .insert("unit".into(), json!("t"));
    w.stockpiles[0].unit = Some(json!("kg"));
    let r = execute_production_cycle(&mut w, &common::request());
    assert!(r.is_err() || r.as_ref().unwrap().outputs[0].quantity == 0.0);
    assert_eq!(w.stockpiles[0].quantity, 1.1);
}
#[test]
fn numeric_and_string_employment_ids_have_independent_paid_work() {
    let mut w = common::world();
    w.pops[0].size = 2;
    w.pops[0].workforce = 2.0;
    w.stockpiles[0].quantity = 2.2;
    w.stockpiles[1].quantity = 0.8;
    w.production_data.factories[0].base.assignments[0].employment_id = Some(7u32.into());
    let mut f = w.production_data.factories[0].clone();
    f.base.id = "g".into();
    f.base.assignments[0].employment_id = Some("7".into());
    w.production_data.factories.push(f);
    let r = execute_production_cycle(&mut w, &common::request()).unwrap();
    assert_eq!(r.outputs.iter().map(|r| r.quantity).sum::<f64>(), 2.0);
    assert_eq!(r.payroll.len(), 2);
    assert_ne!(r.payroll[0].event_id, r.payroll[1].event_id);
}
#[test]
fn prepared_publication_checks_history_conflict_and_matching_replay() {
    for same in [false, true] {
        let mut w = common::world();
        let op: EconomicOperation =
            serde_json::from_value(json!({"event_id":"same","type":"blocked","reason":"a"}))
                .unwrap();
        let mut other = op.clone();
        if !same {
            other.meta.insert("reason".into(), json!("b"));
        }
        let mut s = OperationSession::new(&w).unwrap();
        s.stage(&op).unwrap();
        let first = s.finish().unwrap();
        let mut s = OperationSession::new(&w).unwrap();
        s.stage(&other).unwrap();
        let second = s.finish().unwrap();
        apply_prepared(&mut w, first).unwrap();
        let before = w.clone();
        let result = apply_prepared(&mut w, second);
        assert_eq!(result.is_ok(), same);
        assert_eq!(w, before);
    }
}
