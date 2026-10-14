#[path = "common/labor.rs"]
mod common;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::*;
use kingmaker_sim::local_hiring::plan_artisan_hiring;
use serde_json::json;
fn fixture() -> WorldState {
    let mut w = production::world();
    w.pops[0].pop_type_id = "artisans".into();
    w.pops[0].size = 10;
    w.pops[0].workforce = 10.;
    w.pops[0].employed = 0.;
    let mut a = serde_json::to_value(&w.production_data.factories[0]).unwrap();
    a["assignments"][0]["time_fraction"] = json!(0);
    a["people_slots"] = json!(10);
    w.production_data
        .artisans
        .push(serde_json::from_value(a).unwrap());
    w.production_data.factories.clear();
    let m = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
    m.kind = "artisan".into();
    m.staffing = vec!["artisans".into()];
    w.stockpiles[0].quantity = 5.5;
    w.stockpiles[1].quantity = 2.;
    w
}
#[test]
fn own_artisan_resumes_from_real_inputs_without_salary_emission() {
    let w = fixture();
    let before = serde_json::to_value(&w).unwrap();
    let plan = plan_artisan_hiring(&w, "w").unwrap();
    let source = common::oracle(json!({"action":"artisan_hiring","snapshot":common::snapshot(&w)}));
    assert_eq!(plan.hired_fte(), 5.);
    assert_eq!(
        plan.hired_fte(),
        source["result"]["hired_fte"].as_f64().unwrap()
    );
    assert!(plan.operations.is_empty());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn retooling_missing_technology_and_no_inputs_block_artisan_resume() {
    for case in 0..3 {
        let mut w = fixture();
        if case == 0 {
            w.production_data.artisans[0].pending_retooling = Some(json!(true));
        }
        if case == 1 {
            w.production_data.catalog.as_mut().unwrap().methods[0]
                .technology
                .push("tech".into());
        }
        if case == 2 {
            w.stockpiles[0].quantity = 0.;
        }
        assert_eq!(plan_artisan_hiring(&w, "w").unwrap().hired_fte(), 0.);
    }
}
