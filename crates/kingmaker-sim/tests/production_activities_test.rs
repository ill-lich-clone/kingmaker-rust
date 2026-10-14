#[path = "common/production.rs"]
mod common;
use kingmaker_core::production::*;
use kingmaker_sim::production_activities::*;
use serde_json::json;
#[test]
fn rgo_uses_authored_resource_capacity_and_species() {
    let mut w = common::world();
    w.production_data.factories.clear();
    w.pops[0].pop_type_id = "farmers".into();
    w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"rgo:iron","kind":"rgo","status":"ready","output_goods_id":"iron","output_unit":"t","weekly_productivity":2,"inputs":[],"staffing":["farmers","slaves"]})).unwrap());
    w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"r","owner_id":"o","province_id":1,"method_id":"rgo:iron","resource_id":"iron","resource_status":"approved","approval_ref":"map","land_access":true,"initial_multiplier":3,"assignments":[{"pop_id":"p","workers_count":1,"time_fraction":1,"productivity_multiplier":0.5}]})).unwrap());
    assert_eq!(
        rgo_capacity(
            &w.production_data.resource_operations[0],
            &w.production_data.expansions
        )
        .unwrap(),
        120000.0
    );
    assert_eq!(
        plan_production_activities(&w, &common::request()).unwrap()[0].output,
        1.0
    );
    w.production_data.resource_operations[0].base.assignments[0].productivity_status =
        Some("unknown".into());
    assert_eq!(
        plan_production_activities(&w, &common::request()).unwrap()[0].output,
        0.0
    );
    w.production_data.resource_operations[0].resource_id = "steel".into();
    assert!(plan_production_activities(&w, &common::request()).is_err());
}
#[test]
fn factory_preserves_equipment_clerks_tenure_infrastructure() {
    let mut w = common::world();
    let mut clerk = w.pops[0].clone();
    clerk.external_id = "clerk".into();
    clerk.pop_type_id = "clerks".into();
    w.pops.push(clerk);
    w.production_data.factories[0].base.assignments.push(
        serde_json::from_value(json!({"pop_id":"clerk","workers_count":1,"time_fraction":0.5}))
            .unwrap(),
    );
    let f = &mut w.production_data.factories[0];
    f.tenure_days = Some(15.0);
    f.staff_count = Some(2.0);
    f.parameters=Some(serde_json::from_value(json!({"status":"source","source":"author","clerk_Bmax":0.2,"clerk_ratio":0.5,"tenure_Bmax":0.4,"tenure_Tsat":30})).unwrap());
    w.production_data.completed_construction.push(serde_json::from_value(json!({"status":"completed","province_id":1,"level":1,"effects":{"factory_output_multiplier":1.08}})).unwrap());
    let a = plan_production_activities(&w, &common::request())
        .unwrap()
        .remove(0);
    assert!((a.output - 1.512).abs() < 1e-10);
    w.production_data.factories[0].equipment_capacity_weekly = Some(1.0);
    assert!(
        (plan_production_activities(&w, &common::request()).unwrap()[0].output - 1.08).abs()
            < 1e-10
    );
    let updated = update_factory_tenure(&w.production_data.factories[0], 1.0).unwrap();
    assert_eq!(updated.tenure_days, Some(18.5));
    assert_eq!(w.production_data.factories[0].tenure_days, Some(15.0));
}
#[test]
fn artisan_independence_and_total_assignment_limits() {
    let mut w = common::world();
    w.production_data.factories.clear();
    w.pops[0].pop_type_id = "artisans".into();
    let m = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
    m.kind = "artisan".into();
    m.staffing = vec!["artisans".into()];
    m.method_id = "artisan:steel".into();
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"a","owner_id":"o","province_id":1,"method_id":"artisan:steel","people_slots":1,"assignments":[{"pop_id":"p","workers_count":1,"time_fraction":1}]})).unwrap());
    let a = plan_production_activities(&w, &common::request())
        .unwrap()
        .remove(0);
    assert_eq!(a.output, 1.0);
    assert!(!a.payroll_required);
    let rows:Vec<LaborAssignment>=serde_json::from_value(json!([{"pop_id":"p","workers_count":1,"time_fraction":0.75},{"pop_id":"p","workers_count":1,"time_fraction":0.75}])).unwrap();
    assert!(validate_production_assignments(&w.pops, &rows).is_err());
    w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"c","status":"confirmed","assignments":[{"pop_id":"p","workers_count":1,"time_fraction":0.1}]})).unwrap());
    assert!(plan_production_activities(&w, &common::request()).is_err());
}
