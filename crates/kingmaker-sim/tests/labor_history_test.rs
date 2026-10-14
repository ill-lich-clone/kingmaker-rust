#[path = "common/budget.rs"]
mod budget;
#[path = "common/labor.rs"]
mod common;
use kingmaker_core::{operations::*, *};
use kingmaker_sim::labor_history::plan_unemployment_history;
use serde_json::{json, Value};
fn pay(id: &str, amount: i64) -> EconomicOperation {
    serde_json::from_value(json!({"event_id":format!("pay:{id}"),"type":"payroll","obligation":null,"employment_id":id,"paid_gross_minor":amount.to_string()})).unwrap()
}
fn source(w: &WorldState, p: &str, ops: &[EconomicOperation]) -> Value {
    let mut snapshot = common::snapshot(w);
    snapshot["registry"]["popMobility"] = w.labor_data.pop_mobility.clone().unwrap_or(json!({}));
    snapshot["registry"]["constructionProjects"] = json!(w.production_data.construction_projects);
    common::oracle(json!({"action":"history","snapshot":snapshot,"period":p,"payroll":ops}))
        ["popMobility"]
        .clone()
}
#[test]
fn two_real_unpaid_periods_release_work_but_keep_the_existing_wage_claim() {
    let mut w = budget::world(0);
    w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"old-wage","payer_id":"s","payee_id":"teacher","currency_id":"X","purpose":"state_service:education","amount_minor":"123","employment_id":"teacher"})).unwrap());
    let claims = w.production_data.obligations.clone();
    w.labor_data.pop_mobility = Some(plan_unemployment_history(&w, "w1", &[]).unwrap());
    w.labor_data.pop_mobility = Some(plan_unemployment_history(&w, "w2", &[]).unwrap());
    assert_eq!(
        w.labor_data.pop_mobility.as_ref().unwrap()["unpaid_weeks"]["teacher"],
        json!(2)
    );
    let released = kingmaker_sim::labor_release::release_employment(&w, "w3").unwrap();
    assert_eq!(released.released_fte, 2.);
    released.apply(&mut w).unwrap();
    assert_eq!(w.production_data.obligations, claims);
    assert!(w.pops.iter().all(|p| p.employed == 0.));
}
#[test]
fn oldest_free_people_age_and_newly_free_people_start_at_one() {
    let mut w = common::world(0);
    w.production_data.resource_operations[0].base.assignments = serde_json::from_value(
        json!([{"employment_id":"worked","pop_id":"a","workers_count":300,"time_fraction":1}]),
    )
    .unwrap();
    w.labor_data.pop_mobility = Some(
        json!({"unemployment":{"a":[{"workers":200,"weeks":1},{"workers":200,"weeks":4}]},"remainders":{"a":{"profession":0.3}},"changed_period":{"a":"old"},"custom":"keep"}),
    );
    let actual = plan_unemployment_history(&w, "w", &[]).unwrap();
    assert_eq!(actual, source(&w, "w", &[]));
    assert_eq!(
        actual["unemployment"]["a"],
        json!([{"workers":200,"weeks":5},{"workers":100,"weeks":2}])
    );
    assert_eq!(actual["custom"], json!("keep"));
    assert_eq!(actual["remainders"]["a"]["profession"], json!(0.3));
    let mut w = common::world(0);
    w.labor_data.pop_mobility = Some(json!({"unemployment":{"a":[{"workers":200,"weeks":4}]}}));
    let actual = plan_unemployment_history(&w, "w", &[]).unwrap();
    assert_eq!(
        actual["unemployment"]["a"],
        json!([{"workers":200,"weeks":5},{"workers":400,"weeks":1}])
    );
}
#[test]
fn transit_preserves_unemployment_and_repeat_period_does_not_age() {
    let mut w = common::world(0);
    w.pops[0]
        .extra
        .insert("mobility_status".into(), json!("in_transit"));
    w.labor_data.pop_mobility =
        Some(json!({"unemployment":{"a":[{"workers":600,"weeks":3}]},"unpaid_weeks":{"old":2}}));
    let actual = plan_unemployment_history(&w, "w", &[]).unwrap();
    assert_eq!(actual, source(&w, "w", &[]));
    assert_eq!(
        actual["unemployment"]["a"],
        json!([{"workers":600,"weeks":3}])
    );
    w.labor_data.pop_mobility = Some(actual.clone());
    assert_eq!(
        plan_unemployment_history(&w, "w", &[pay("irrelevant", 1)]).unwrap(),
        actual
    );
}
#[test]
fn actual_positive_partial_pay_resets_unpaid_while_zero_and_no_pay_age() {
    let mut w = budget::world(0);
    w.labor_data.pop_mobility = Some(json!({"unpaid_weeks":{"teacher":2,"medic":3,"ended":20}}));
    let ops = [pay("teacher", 1), pay("medic", 0)];
    let actual = plan_unemployment_history(&w, "w", &ops).unwrap();
    assert_eq!(actual, source(&w, "w", &ops));
    assert_eq!(actual["unpaid_weeks"], json!({"teacher":0,"medic":4}));
    assert!(actual["unemployment"]["teacher"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn ended_and_inactive_builder_jobs_are_not_unpaid_claims() {
    let mut w = budget::world(0);
    w.budget_data.employment[0]
        .extra
        .insert("status".into(), json!("ended"));
    w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"project","status":"awaiting_funding","assignments":[{"employment_id":"medic","pop_id":"medic","workers_count":1,"time_fraction":1,"contract_id":"medic"}]})).unwrap());
    let actual = plan_unemployment_history(&w, "w", &[]).unwrap();
    assert_eq!(actual, source(&w, "w", &[]));
    assert!(actual["unpaid_weeks"].as_object().unwrap().is_empty());
    assert_eq!(actual["unemployment"]["teacher"][0]["weeks"], json!(1));
}
#[test]
fn invalid_history_or_negative_pay_reject_without_mutating_source() {
    for case in 0..3 {
        let mut w = budget::world(0);
        w.labor_data.pop_mobility = Some(match case {
            0 => json!({"unemployment":{"teacher":[{"workers":-1,"weeks":3}]}}),
            1 => json!({"unpaid_weeks":{"teacher":-1}}),
            _ => json!({}),
        });
        let before = serde_json::to_value(&w).unwrap();
        let ops = if case == 2 {
            vec![pay("teacher", -1)]
        } else {
            vec![]
        };
        assert!(plan_unemployment_history(&w, "w", &ops).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
