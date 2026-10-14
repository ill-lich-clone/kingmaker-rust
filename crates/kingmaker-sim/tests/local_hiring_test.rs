#[path = "common/labor.rs"]
mod common;
use kingmaker_core::{production::*, *};
use kingmaker_sim::local_hiring::*;
use serde_json::json;
#[path = "common/production.rs"]
mod production;
#[test]
fn factory_hires_only_real_recipe_and_preserves_source_tenure() {
    let mut w = production::world();
    let c = common::world(0);
    w.market_data.needs = c.market_data.needs;
    w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = "p".into();
    w.market_data.price_calibration = c.market_data.price_calibration;
    w.pops[0].size = 10;
    w.pops[0].workforce = 10.;
    w.pops[0].employed = 0.;
    w.pops[0]
        .extra
        .insert("species_labor_coefficient".into(), json!(1));
    w.production_data.factories[0].base.assignments.clear();
    w.production_data.factories[0].staff_count = Some(0.);
    w.production_data.contracts.clear();
    w.stockpiles[0].quantity = 5.5;
    w.stockpiles[1].quantity = 2.;
    let plan = plan_local_hiring(&w, "w", &ProductionPolicies::default()).unwrap();
    let source = common::oracle(json!({"action":"hiring","snapshot":common::snapshot(&w)}));
    assert_eq!(plan.hired_fte(), 5.);
    assert_eq!(
        plan.hired_fte(),
        source["result"]["hired_fte"].as_f64().unwrap()
    );
    assert_eq!(
        plan.contracts()[0]
            .weekly_wage_rate
            .as_ref()
            .unwrap()
            .numerator,
        source["registry"]["contracts"][0]["weekly_wage_rate"]["numerator"]
    );
}
#[test]
fn numeric_and_string_pop_ids_get_separate_assignments() {
    let mut w = common::world(100000);
    w.pops[0].external_id = serde_json::from_value(json!(7)).unwrap();
    w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = w.pops[0].external_id.clone();
    let mut p = w.pops[0].clone();
    p.id = PopId(5);
    p.external_id = "7".into();
    w.pops.push(p);
    let mut r = w.market_data.needs.as_ref().unwrap().profile_refs[0].clone();
    r.pop_id = "7".into();
    w.market_data.needs.as_mut().unwrap().profile_refs.push(r);
    let plan = plan_local_hiring(&w, "w", &ProductionPolicies::default()).unwrap();
    assert_eq!(plan.hired_fte(), 1200.);
    assert_eq!(plan.employment().len(), 2);
    assert_ne!(
        plan.employment()[0].employment_id,
        plan.employment()[1].employment_id
    );
    assert_eq!(plan.contracts().len(), 2);
}
#[test]
fn free_local_farmer_gets_work_now_with_exact_funded_own_salary() {
    for money in [100000, 1540, 0] {
        let w = common::world(money);
        let before = serde_json::to_value(&w).unwrap();
        let p = plan_local_hiring(&w, "w", &ProductionPolicies::default()).unwrap();
        let source = common::oracle(json!({"action":"hiring","snapshot":common::snapshot(&w)}));
        assert_eq!(
            p.hired_fte(),
            source["result"]["hired_fte"].as_f64().unwrap()
        );
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
        assert_eq!(p.contracts().len(), if money == 0 { 0 } else { 1 });
        if money > 0 {
            assert_eq!(
                p.contracts()[0]
                    .weekly_wage_rate
                    .as_ref()
                    .unwrap()
                    .numerator,
                "77"
            );
            assert_eq!(p.pops()[0].employed, p.hired_fte());
        }
    }
}
#[test]
fn concurrent_pops_share_money_and_keep_distinct_own_rates() {
    let mut w = common::world(1540);
    let mut p = w.pops[0].clone();
    p.id = PopId(5);
    p.external_id = "q".into();
    p.size = 500;
    p.workforce = 300.;
    w.pops.push(p);
    let mut r = w.market_data.needs.as_ref().unwrap().profile_refs[0].clone();
    r.pop_id = "q".into();
    w.market_data.needs.as_mut().unwrap().profile_refs.push(r);
    let plan = plan_local_hiring(&w, "w", &ProductionPolicies::default()).unwrap();
    let source = common::oracle(json!({"action":"hiring","snapshot":common::snapshot(&w)}));
    assert_eq!(
        plan.hired_fte(),
        source["result"]["hired_fte"].as_f64().unwrap()
    );
    assert_eq!(plan.employment().len(), 2);
    for e in plan.employment() {
        let j = source["registry"]["employment"]
            .as_array()
            .unwrap()
            .iter()
            .find(|j| j["pop_id"] == e.pop_id.json())
            .unwrap();
        assert_eq!(e.fte, j["fte"].as_f64().unwrap());
    }
    assert_eq!(
        plan.operations.len(),
        source["operations"].as_array().unwrap().len()
    );
}
#[test]
fn blocked_better_named_factory_releases_quota_to_real_rgo_in_same_phase() {
    let mut w = common::world(100000);
    let m = w.production_data.catalog.as_mut().unwrap();
    m.methods.push(serde_json::from_value(json!({"method_id":"factory:test","kind":"factory","status":"ready","output_goods_id":"grain","output_unit":"t","weekly_productivity":1,"staffing":["farmers"],"inputs":[{"goods_id":"missing","amount":1}]})).unwrap());
    w.production_data.factories.push(serde_json::from_value(json!({"id":"a","owner_id":"o","province_id":1,"method_id":"factory:test","facility_id":"wh","status":"operating","people_slots":40000,"capacity_source":"fixture","equipment_capacity_weekly":100,"equipment_source":"fixture","staff_count":0,"tenure_days":0,"assignments":[]})).unwrap());
    let p = plan_local_hiring(&w, "w", &ProductionPolicies::default()).unwrap();
    assert_eq!(p.hired_fte(), 600.);
    assert!(p.hires.iter().all(|h| h["entity_id"] == "rgo"));
}
#[test]
fn sorted_money_results_are_joined_by_bid_id_for_unequal_unsorted_pops() {
    let mut w = common::world(0);
    w.pops[0].external_id = "z".into();
    w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = "z".into();
    let mut p = w.pops[0].clone();
    p.id = PopId(5);
    p.external_id = "a".into();
    p.size = 500;
    p.workforce = 300.;
    w.pops.push(p);
    let mut r = w.market_data.needs.as_ref().unwrap().profile_refs[0].clone();
    r.pop_id = "a".into();
    w.market_data.needs.as_mut().unwrap().profile_refs.push(r);
    w.accounts.push(production::account(
        102,
        "o",
        "employer_operating_capital",
        4620,
    ));
    w.rebuild_account_index().unwrap();
    let plan = plan_local_hiring(&w, "w", &ProductionPolicies::default()).unwrap();
    let source = common::oracle(json!({"action":"hiring","snapshot":common::snapshot(&w)}));
    assert_eq!(plan.hired_fte(), 900.);
    assert_eq!(
        plan.hired_fte(),
        source["result"]["hired_fte"].as_f64().unwrap()
    );
}
