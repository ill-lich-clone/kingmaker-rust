#[path = "common/production.rs"]
mod production;
use kingmaker_sim::{labor_offers::build_job_offers, labor_roster::build_labor_roster};
use serde_json::json;
#[path = "common/budget.rs"]
mod budget;
#[test]
fn real_service_offers_respect_operational_and_legal_conditions() {
    let mut w = budget::world(0);
    w.labor_data.profiles.push(json!({"state_id":"s"}));
    w.budget_data.jobs[0]["building_id"] = json!("school");
    w.budget_data.jobs[0]["role_id"] = json!("teacher");
    w.budget_data.jobs[0]["pop_type_id"] = json!("clergy");
    w.budget_data.jobs[0]["slots_total"] = json!(2);
    w.market_data.institutions.push(json!({"building_id":"school","building_type_id":"school","province_id":1,"owner_entity_id":"s","operational_status":"active"}));
    let r = build_labor_roster(&w).unwrap();
    let result = build_job_offers(&w, &r).unwrap();
    assert_eq!(result.offers.len(), 1);
    assert_eq!(result.offers[0].kind, "service");
    assert_eq!(result.offers[0].places, 1);
    w.market_data.institutions[0]["operational_status"] = json!("closed");
    assert!(build_job_offers(&w, &r).unwrap().offers.is_empty());
}
#[test]
fn falsy_equipment_provenance_does_not_authorize_a_factory_offer() {
    for value in [json!(false), json!("")] {
        let mut w = production::world();
        w.production_data.factories[0].equipment_source = Some(value);
        let r = build_labor_roster(&w).unwrap();
        let result = build_job_offers(&w, &r).unwrap();
        assert!(result.offers.is_empty());
    }
}
#[test]
fn factory_vacancy_requires_real_equipment_technology_and_free_heads() {
    let mut w = production::world();
    let r = build_labor_roster(&w).unwrap();
    let offers = build_job_offers(&w, &r).unwrap();
    assert_eq!(offers.offers.len(), 1);
    assert_eq!(offers.offers[0].places, 39999);
    w.production_data.factories[0].equipment_capacity_weekly = Some(0.);
    let offers = build_job_offers(&w, &r).unwrap();
    assert!(offers.offers.is_empty());
    assert_eq!(
        offers.blocked[0]["reason"],
        "factory_equipment_or_technology_unavailable"
    );
}
