#[path = "common/budget.rs"]
mod common;
use common::*;
use kingmaker_core::{operations::*, *};
use kingmaker_sim::budget_cycle::execute_budget_cycle;
use serde_json::json;
#[path = "common/production.rs"]
mod production_fixture;

#[test]
fn shared_capacity_pool_primary_profession_does_not_override_specific_contract() {
    let mut w = world(4000);
    w.production_data.contracts[0]
        .extra
        .insert("pop_type_id".into(), json!("clergymen"));
    w.budget_data.jobs[0]["pop_type_id"] = json!("bureaucrats");
    w.budget_data.jobs[0]["profile_kind"] = json!("capacity_pool");
    let r = request(&w);
    execute_budget_cycle(&mut w, &r).unwrap();
    assert_eq!(cash(&w, "teacher", "personal_savings"), 900);
}

#[test]
fn supplied_malformed_workers_or_time_are_not_defaulted() {
    for field in ["workers_count", "time_fraction"] {
        let mut w = world(4000);
        w.budget_data.employment[0]
            .extra
            .insert(field.into(), json!("1"));
        let before = w.clone();
        let r = request(&w);
        assert!(execute_budget_cycle(&mut w, &r).is_err());
        assert_eq!(w, before);
    }
}
#[test]
fn missing_actual_job_or_authored_wrong_profession_cannot_receive_service_pay() {
    for mismatch in [false, true] {
        let mut w = world(4000);
        if mismatch {
            w.budget_data.jobs[0]["pop_type_id"] = json!("bureaucrats");
        } else {
            w.budget_data.jobs.remove(0);
        }
        let before = w.clone();
        let r = request(&w);
        assert!(execute_budget_cycle(&mut w, &r).is_err());
        assert_eq!(w, before);
    }
}

#[test]
fn numeric_and_string_employment_ids_do_not_collapse_payroll_events() {
    let mut w = world(4000);
    w.budget_data.employment[0].employment_id = ExternalId::Number(7.into());
    w.budget_data.employment[1].employment_id = "7".into();
    let r = request(&w);
    let receipt = execute_budget_cycle(&mut w, &r).unwrap();
    let payroll = receipt
        .operations
        .iter()
        .filter(|o| o.kind == OperationKind::Payroll)
        .collect::<Vec<_>>();
    assert_eq!(payroll.len(), 2);
    assert_ne!(payroll[0].event_id, payroll[1].event_id);
    assert_eq!(cash(&w, "teacher", "personal_savings"), 900);
    assert_eq!(cash(&w, "medic", "personal_savings"), 900);
}

#[test]
fn existing_production_assignment_is_excluded_from_service_payroll() {
    let mut w = production_fixture::world();
    w.budget_data.employment.push(serde_json::from_value(json!({"employment_id":"e","pop_id":"p","contract_id":"c","fte":1,"workers_count":1,"time_fraction":1})).unwrap());
    let r = request(&w);
    let receipt = execute_budget_cycle(&mut w, &r).unwrap();
    assert!(!receipt
        .operations
        .iter()
        .any(|o| o.kind == OperationKind::Payroll));
    assert_eq!(w.accounts[0].balance, Money(20));
}

#[test]
fn strict_initial_plan_blocks_and_does_not_create_wage_debt() {
    let mut w = world(4000);
    w.budget_data.profile_id = Some("strict".into());
    w.budget_data.employment[0]
        .extra
        .insert("building_id".into(), json!("school"));
    w.market_data
        .institutions
        .push(json!({"building_id":"school","operational_status":"initial_plan"}));
    let r = request(&w);
    let receipt = execute_budget_cycle(&mut w, &r).unwrap();
    let teacher = receipt
        .operations
        .iter()
        .find(|o| o.meta.get("pop_id") == Some(&json!("teacher")))
        .unwrap();
    assert_eq!(teacher.kind, OperationKind::Blocked);
    assert_eq!(teacher.meta["reason"], json!("initial_plan_unconfirmed"));
    assert!(w
        .production_data
        .obligations
        .iter()
        .all(|d| d.payee_id != ExternalId::from("teacher")));
}
#[test]
fn sliders_change_request_not_contract_and_zero_request_has_no_paid_labor_or_debt() {
    for (pct, gross) in [(0., 0), (50., 500), (100., 1000)] {
        let mut w = world(4000);
        funding(&mut w, pct, 0.);
        let rate = w.production_data.contracts[0].weekly_wage_rate.clone();
        let r = request(&w);
        let receipt = execute_budget_cycle(&mut w, &r).unwrap();
        let op = receipt
            .operations
            .iter()
            .find(|o| o.kind == OperationKind::Payroll && o.meta["pop_id"] == json!("teacher"))
            .unwrap();
        assert_eq!(op.meta["gross_minor"], json!(gross.to_string()));
        assert_eq!(
            op.meta["actual_fte"],
            json!(if gross > 0 { 1. } else { 0. })
        );
        assert!(op.obligation.is_none());
        assert_eq!(w.production_data.contracts[0].weekly_wage_rate, rate);
    }
}
#[test]
fn zero_slider_preserves_prior_debt_and_other_earmarked_accounts() {
    let mut w = world(500);
    funding(&mut w, 0., 0.);
    w.accounts
        .push(account(11, "s", "X", "treasury_operating_capital", 9999));
    w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"old","payer_id":"s","currency_id":"X","purpose":"state_service:education","payee_id":"teacher","pop_id":"teacher","amount_minor":"500","budget_state_id":"s","budget_sector":"education"})).unwrap());
    w.rebuild_account_index().unwrap();
    let r = request(&w);
    execute_budget_cycle(&mut w, &r).unwrap();
    assert_eq!(w.production_data.obligations[0].amount_minor, Money::ZERO);
    assert_eq!(cash(&w, "s", "treasury_operating_capital"), 9999);
    assert_eq!(cash(&w, "teacher", "personal_savings"), 500);
}
#[test]
fn excessive_workforce_and_job_capacity_fail_without_payments() {
    for kind in 0..2 {
        let mut w = world(2000);
        if kind == 0 {
            w.pops[0].workforce = 0.5;
        } else {
            w.budget_data.jobs[0]["slots_total"] = json!(0);
        }
        let before = w.clone();
        let r = request(&w);
        assert!(execute_budget_cycle(&mut w, &r).is_err());
        assert_eq!(w, before);
    }
}
#[test]
fn direct_output_and_slaves_do_not_receive_service_wages() {
    let mut w = world(4000);
    w.budget_data.employment[0]
        .extra
        .insert("direct_output_labor".into(), json!(true));
    w.pops[1].pop_type_id = "slaves".into();
    let r = request(&w);
    let receipt = execute_budget_cycle(&mut w, &r).unwrap();
    assert!(!receipt
        .operations
        .iter()
        .any(|o| o.kind == OperationKind::Payroll));
    assert_eq!(cash(&w, "teacher", "personal_savings"), 0);
    assert_eq!(cash(&w, "medic", "personal_savings"), 0);
    assert!(receipt
        .operations
        .iter()
        .any(|o| o.kind == OperationKind::Blocked));
}
#[test]
fn archived_service_has_no_pay_or_new_budget_reservation() {
    let mut w = common::world(4000);
    w.budget_data.employment[0]
        .extra
        .insert("status".into(), serde_json::json!("ended"));
    let r = common::request(&w);
    let receipt = kingmaker_sim::budget_cycle::execute_budget_cycle(&mut w, &r).unwrap();
    assert!(!receipt.operations.iter().any(|o| o
        .credits
        .iter()
        .any(|leg| leg.owner_id == kingmaker_core::ExternalId::from("teacher"))));
}
#[test]
fn zero_fte_historical_profession_does_not_block_real_payroll() {
    let mut w = common::world(4000);
    w.budget_data.employment[0].fte = 0.;
    w.budget_data.employment[0]
        .extra
        .insert("workers_count".into(), serde_json::json!(0));
    w.pops[0].pop_type_id = "artisans".into();
    w.production_data.contracts[0]
        .extra
        .insert("pop_type_id".into(), serde_json::json!("clergymen"));
    let r = common::request(&w);
    let p = kingmaker_sim::budget_cycle::execute_budget_cycle(&mut w, &r).unwrap();
    assert!(!p.operations.iter().any(|o| o
        .credits
        .iter()
        .any(|c| c.owner_id == kingmaker_core::ExternalId::from("teacher"))));
    assert_eq!(w.budget_data.employment.len(), 2);
}
