#[path = "common/labor.rs"]
mod common;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::{labor::LaborRequest, production::*, *};
use kingmaker_sim::{
    labor_cycle::execute_local_labor, labor_release::release_employment,
    labor_roster::build_labor_roster, local_hiring::plan_local_hiring,
    production::execute_production_cycle,
};
use serde_json::json;
#[test]
fn partial_typed_fte_release_is_accepted_by_real_production() {
    let mut w = production::world();
    w.pops[0].size = 10;
    w.pops[0].workforce = 10.;
    let f = &mut w.production_data.factories[0];
    f.people_slots = Some(5.);
    f.staff_count = Some(10.);
    f.base.assignments[0].workers_count = 10.;
    f.base.assignments[0].fte = Some(10.);
    w.stockpiles[0].quantity = 5.5;
    w.stockpiles[1].quantity = 2.;
    w.accounts[0].balance = Money(100);
    let changes = release_employment(&w, "w").unwrap();
    changes.apply(&mut w).unwrap();
    assert_eq!(
        w.production_data.factories[0].base.assignments[0].fte,
        Some(5.)
    );
    let result = execute_production_cycle(&mut w, &production::request()).unwrap();
    assert_eq!(result.outputs[0].quantity, 5.);
}
#[test]
fn malformed_typed_assignment_fte_is_rejected_by_shared_roster() {
    let mut w = production::world();
    w.production_data.factories[0].base.assignments[0].fte = Some(0.5);
    assert!(build_labor_roster(&w).is_err());
}
#[test]
fn inactive_builder_can_take_real_factory_work_without_double_production_labor() {
    for status in ["awaiting_funding", "awaiting_owner_consent"] {
        let mut w = production::world();
        let c = common::world(0);
        w.market_data.needs = c.market_data.needs;
        w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = "p".into();
        w.market_data.price_calibration = c.market_data.price_calibration;
        w.pops[0]
            .extra
            .insert("species_labor_coefficient".into(), json!(1));
        w.production_data.factories[0].base.assignments.clear();
        w.production_data.factories[0].staff_count = Some(0.);
        w.production_data.contracts.clear();
        w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"project","status":status,"assignments":[{"employment_id":"builder","pop_id":"p","workers_count":1,"time_fraction":1}]})).unwrap());
        let r = LaborRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        };
        let p = execute_local_labor(&mut w, &r).unwrap();
        assert_eq!(p.hired_fte, 1.);
        kingmaker_sim::budget_cycle::execute_budget_cycle(
            &mut w,
            &kingmaker_core::budget::BudgetRequest {
                expected_revision: 20,
                period: "w".into(),
                policies: Default::default(),
            },
        )
        .unwrap();
        let result = execute_production_cycle(&mut w, &production::request()).unwrap();
        assert_eq!(result.outputs[0].quantity, 1.);
    }
}
fn groups() -> WorldState {
    let mut w = production::world();
    w.pops[0].size = 10;
    w.pops[0].workforce = 10.;
    let f = &mut w.production_data.factories[0];
    f.staff_count = Some(5.);
    f.base.assignments[0].workers_count = 5.;
    f.base.assignments[0].time_fraction = 0.75;
    f.base.assignments[0].worker_group_id = Some("g".into());
    let mut second = f.clone();
    second.base.id = "second".into();
    second.base.assignments[0].employment_id = Some("e2".into());
    w.production_data.factories.push(second);
    w
}
#[test]
fn typed_worker_group_rejects_double_time_and_keeps_valid_and_distinct_groups() {
    let mut w = groups();
    assert!(build_labor_roster(&w).is_err());
    for f in &mut w.production_data.factories {
        f.base.assignments[0].time_fraction = 0.5;
    }
    assert!(build_labor_roster(&w).is_ok());
    w.production_data.factories[0].base.assignments[0].time_fraction = 0.75;
    w.production_data.factories[1].base.assignments[0].time_fraction = 0.75;
    w.production_data.factories[0].base.assignments[0].worker_group_id =
        Some(serde_json::from_value(json!(7)).unwrap());
    w.production_data.factories[1].base.assignments[0].worker_group_id = Some("7".into());
    assert!(build_labor_roster(&w).is_ok());
}
#[test]
fn explicit_mirror_contract_conflict_rejects_without_rewriting_author_terms() {
    let mut w = production::world();
    let mut c = w.production_data.contracts[0].clone();
    c.contract_id = "other".into();
    c.weekly_wage_rate.as_mut().unwrap().numerator = "999".into();
    w.production_data.contracts.push(c);
    w.budget_data.employment.push(serde_json::from_value(json!({"employment_id":"e","pop_id":"p","contract_id":"other","workers_count":1,"time_fraction":1,"fte":1})).unwrap());
    let before = serde_json::to_value(&w).unwrap();
    assert!(build_labor_roster(&w).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    w.production_data.factories[0].base.assignments[0].contract_id = None;
    assert_eq!(
        build_labor_roster(&w).unwrap().by_employment[&ExternalId::from("e")].contract_id,
        Some("other".into())
    );
}
#[test]
fn typed_service_job_ids_produce_separate_quota_contract_and_employment() {
    let mut w = common::world(0);
    w.pops[0].pop_type_id = "clergy".into();
    w.labor_data.profiles.push(json!({"state_id":"s"}));
    let mut treasury = w.accounts.last().unwrap().clone();
    treasury.id = AccountId(200);
    treasury.external_id = "treasury".into();
    treasury.owner_id = "treasury:s".into();
    treasury.balance = Money(100000);
    w.accounts.push(treasury);
    for (i, id) in [json!(7), json!("7")].iter().enumerate() {
        let building = format!("school{i}");
        w.market_data.institutions.push(json!({"building_id":building,"building_type_id":"school","province_id":1,"owner_entity_id":"s","operational_status":"active"}));
        w.budget_data.jobs.push(json!({"job_id":id,"building_id":building,"province_id":1,"role_id":"teacher","pop_type_id":"clergy","slots_total":1,"capacity_pool_id":id,"capacity_pool_total":1}));
    }
    w.rebuild_account_index().unwrap();
    let p = plan_local_hiring(&w, "w", &ProductionPolicies::default()).unwrap();
    assert_eq!(p.hired_fte(), 2.);
    assert_eq!(p.contracts().len(), 2);
    assert_eq!(p.employment().len(), 2);
    assert_ne!(
        p.employment()[0].employment_id,
        p.employment()[1].employment_id
    );
    assert_ne!(
        p.contracts()[0].extra["job_id"],
        p.contracts()[1].extra["job_id"]
    );
}
#[test]
fn generated_institution_jobs_and_pools_keep_typed_building_identity() {
    let buildings = vec![
        json!({"building_id":7,"building_type_id":"school","province_id":1,"level":1}),
        json!({"building_id":"7","building_type_id":"school","province_id":1,"level":1}),
    ];
    let rows=kingmaker_sim::labor_capacity::derive_institution_jobs(&buildings,&[],&json!({"school":{"places_per_level":2000,"roles":[{"role_id":"teacher","pop_type_id":"clergy","weight":1}]}})).unwrap();
    assert_ne!(rows[0]["job_id"], rows[1]["job_id"]);
    assert_ne!(rows[0]["capacity_pool_id"], rows[1]["capacity_pool_id"]);
}
