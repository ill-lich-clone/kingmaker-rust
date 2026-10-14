#[path = "common/budget.rs"]
mod budget;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::*;
use kingmaker_sim::labor_roster::{build_labor_roster, reconcile_labor};
use serde_json::json;

#[test]
fn supplied_service_fte_cannot_be_silently_recomputed_from_conflicting_time() {
    let mut w = budget::world(0);
    w.budget_data.employment[0]
        .extra
        .insert("time_fraction".into(), json!(0.5));
    assert!(build_labor_roster(&w).is_err());
}
#[test]
fn failed_reconciliation_does_not_publish_earlier_counters() {
    let mut w = budget::world(0);
    let r = build_labor_roster(&w).unwrap();
    w.pops[0].employed = 0.;
    w.pops[0].unemployed = 1.;
    w.budget_data.jobs.push(json!({"slots_total":1}));
    let before = w.clone();
    assert!(reconcile_labor(&mut w, &r).is_err());
    assert_eq!(w, before);
}
#[test]
fn mirrored_production_employment_counts_once_and_reconciles_actual_slots() {
    let mut w = production::world();
    w.budget_data.jobs.push(
        json!({"job_id":"job","slots_total":2,"capacity_pool_id":"job","capacity_pool_total":2}),
    );
    w.production_data.contracts[0]
        .extra
        .insert("job_id".into(), json!("job"));
    w.budget_data.employment.push(serde_json::from_value(json!({"employment_id":"e","pop_id":"p","contract_id":"c","job_id":"job","fte":1,"workers_count":1,"time_fraction":1})).unwrap());
    let roster = build_labor_roster(&w).unwrap();
    assert_eq!(roster.by_pop[&ExternalId::from("p")].occupied, 1.);
    assert_eq!(roster.by_job[&ExternalId::from("job")], 1.);
    assert_eq!(roster.by_employment.len(), 1);
    w.pops[0].employed = 0.;
    w.pops[0].unemployed = 1.;
    reconcile_labor(&mut w, &roster).unwrap();
    assert_eq!(w.pops[0].employed, 1.);
    assert_eq!(w.pops[0].unemployed, 0.);
    assert_eq!(w.budget_data.jobs[0]["slots_filled"].as_f64(), Some(1.));
}
#[test]
fn duplicate_inconsistent_refs_and_workforce_fail() {
    for kind in 0..4 {
        let mut w = budget::world(0);
        match kind {
            0 => w
                .budget_data
                .employment
                .push(w.budget_data.employment[0].clone()),
            1 => w.budget_data.employment[0].contract_id = "missing".into(),
            2 => w.budget_data.employment[0]
                .extra
                .insert("job_id".into(), json!("missing"))
                .map(|_| ())
                .unwrap_or(()),
            _ => w.pops[0].workforce = 0.5,
        };
        assert!(build_labor_roster(&w).is_err());
    }
}
#[test]
fn shared_pool_and_individual_job_caps_both_enforced() {
    let mut w = budget::world(0);
    for j in &mut w.budget_data.jobs {
        j["capacity_pool_id"] = json!("pool");
        j["capacity_pool_total"] = json!(1.);
    }
    assert!(build_labor_roster(&w).is_err());
    for j in &mut w.budget_data.jobs {
        j["capacity_pool_total"] = json!(2.);
    }
    let r = build_labor_roster(&w).unwrap();
    assert_eq!(r.by_pool[&ExternalId::from("pool")].occupied, 2.);
    w.budget_data.jobs[0]["slots_total"] = json!(0);
    assert!(build_labor_roster(&w).is_err());
}
#[test]
fn ended_service_and_inactive_construction_do_not_take_available_labor() {
    let mut w = budget::world(0);
    w.budget_data.employment[0]
        .extra
        .insert("status".into(), json!("ended"));
    let roster = build_labor_roster(&w).unwrap();
    assert_eq!(roster.by_pop[&ExternalId::from("teacher")].free, 1.);
    assert!(!roster
        .by_employment
        .contains_key(&ExternalId::from("teacher")));
}
