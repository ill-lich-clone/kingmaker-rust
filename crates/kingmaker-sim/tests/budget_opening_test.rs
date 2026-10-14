#[path = "common/budget.rs"]
mod common;
#[path = "common/construction.rs"]
mod construction;
use kingmaker_core::*;
use kingmaker_sim::budget_cycle::{
    execute_budget_cycle, execute_budget_opening, execute_budget_services,
};
use serde_json::json;
#[test]
fn opening_funds_and_issues_once_but_pays_services_after_intervening_systems() {
    let mut w = common::world(4500);
    common::adv(&mut w);
    let r = common::request(&w);
    let before = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let opening = execute_budget_opening(&mut w, &r).unwrap();
    assert_eq!(common::cash(&w, "teacher", "personal_savings"), 0);
    assert_eq!(common::cash(&w, "medic", "personal_savings"), 0);
    assert_eq!(common::cash(&w, "a", "personal_savings"), 1800);
    assert_eq!(common::cash(&w, "s", "state_service:education"), 2000);
    assert_eq!(
        w.accounts.iter().map(|a| a.balance.0).sum::<i64>(),
        before + 2000
    );
    assert!(w.budget_data.phase_receipts.is_empty());
    assert_eq!(w.budget_data.opening_receipts.len(), 1);
    let after = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_budget_opening(&mut w, &r).unwrap(), opening);
    assert_eq!(serde_json::to_value(&w).unwrap(), after);
    let complete = execute_budget_services(&mut w, &r).unwrap();
    assert_eq!(common::cash(&w, "teacher", "personal_savings"), 900);
    assert_eq!(common::cash(&w, "medic", "personal_savings"), 900);
    assert_eq!(common::cash(&w, "s", "state_service:education"), 1000);
    assert_eq!(
        w.accounts.iter().map(|a| a.balance.0).sum::<i64>(),
        before + 2000
    );
    let after = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_budget_services(&mut w, &r).unwrap(), complete);
    assert_eq!(serde_json::to_value(&w).unwrap(), after);
}
#[test]
fn immediate_composition_has_same_real_operations_accounts_and_obligations() {
    let mut staged = common::world(4500);
    common::adv(&mut staged);
    let mut combined = staged.clone();
    let r = common::request(&staged);
    execute_budget_opening(&mut staged, &r).unwrap();
    let phased = execute_budget_services(&mut staged, &r).unwrap();
    let result = execute_budget_cycle(&mut combined, &r).unwrap();
    assert_eq!(staged.accounts, combined.accounts);
    assert_eq!(
        staged.production_data.obligations,
        combined.production_data.obligations
    );
    assert_eq!(phased, result);
}
#[test]
fn empty_opening_has_authentic_boundary_and_lost_or_changed_proof_is_refused() {
    let mut w = WorldState::default();
    w.campaign.campaign_id = "empty".into();
    w.campaign.revision = 20;
    w.campaign.year = 306;
    w.campaign.month = 1;
    w.campaign.day = 1;
    let r = common::request(&w);
    assert!(execute_budget_services(&mut w.clone(), &r).is_err());
    let opening = execute_budget_opening(&mut w, &r).unwrap();
    assert_eq!(opening.operations.len(), 1);
    assert!(opening.operations[0].debits.is_empty());
    assert!(opening.operations[0].credits.is_empty());
    for case in 0..3 {
        let mut bad = w.clone();
        match case {
            0 => bad.budget_data.opening_receipts.clear(),
            1 => bad.budget_data.opening_receipts[0]
                .requirements
                .push(json!({"tampered":true})),
            _ => bad.production_data.committed_operations.clear(),
        };
        let before = serde_json::to_value(&bad).unwrap();
        assert!(execute_budget_services(&mut bad, &r).is_err());
        assert_eq!(serde_json::to_value(&bad).unwrap(), before);
    }
    let mut loaded: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    loaded.rebuild_account_index().unwrap();
    execute_budget_services(&mut loaded, &r).unwrap();
}
#[test]
fn changed_policy_cannot_finalize_another_opening_or_duplicate_income() {
    let mut w = common::world(4500);
    let r = common::request(&w);
    execute_budget_opening(&mut w, &r).unwrap();
    let mut changed = r.clone();
    changed.policies.taxes.insert("s".into(), 0.5);
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_budget_services(&mut w, &changed).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn artisan_choices_follow_authentic_opening_but_precede_service_payroll_and_frozen_basis() {
    let mut w = common::world(4500);
    let r = common::request(&w);
    execute_budget_opening(&mut w, &r).unwrap();
    let income = kingmaker_core::income::IncomeRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    };
    assert!(kingmaker_sim::income_basis::capture_income_opening(&w, &income).is_err());
    let mut bad = w.clone();
    bad.budget_data.opening_receipts[0]
        .requirements
        .push(json!({"tampered":true}));
    assert!(kingmaker_sim::income_cycle::execute_artisan_opening(&mut bad, &income).is_err());
    let choices = kingmaker_sim::income_cycle::execute_artisan_opening(&mut w, &income).unwrap();
    assert!(!choices.input_evidence.is_empty());
    for delete_budget in [false, true] {
        let mut broken = w.clone();
        if delete_budget {
            broken.budget_data.opening_receipts.clear();
        } else {
            broken.income_data.phase_receipts[0].input_evidence.clear();
        }
        assert!(
            kingmaker_sim::income_cycle::execute_artisan_opening(&mut broken, &income).is_err()
        );
    }
    let mut late = common::world(4500);
    execute_budget_cycle(&mut late, &r).unwrap();
    assert!(kingmaker_sim::income_cycle::execute_artisan_opening(&mut late, &income).is_err());
}
#[test]
fn investments_use_opening_and_builder_payroll_follows_early_food_but_not_closing_market() {
    let mut w = construction::opening(7);
    let r = common::request(&w);
    let c = construction::request();
    execute_budget_opening(&mut w, &r).unwrap();
    kingmaker_sim::investments::execute_automatic_investments(&mut w, &c).unwrap();
    execute_budget_services(&mut w, &r).unwrap();
    for stage in [
        kingmaker_core::market::MarketStage::Maintenance,
        kingmaker_core::market::MarketStage::EarlyFood,
    ] {
        kingmaker_sim::market_cycle::execute_market_phase(
            &mut w,
            &kingmaker_core::market::MarketRequest {
                expected_revision: 20,
                period: "w".into(),
                stage,
                profile: Default::default(),
                priorities: None,
            },
        )
        .unwrap();
    }
    let mut late = w.clone();
    kingmaker_sim::production::execute_production_cycle(
        &mut late,
        &kingmaker_core::production::ProductionRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
            profile: Default::default(),
            maintenance_receipts: vec![],
        },
    )
    .unwrap();
    kingmaker_sim::market_cycle::execute_market_phase(
        &mut late,
        &kingmaker_core::market::MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: kingmaker_core::market::MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: None,
        },
    )
    .unwrap();
    assert!(
        kingmaker_sim::construction_payroll::execute_construction_payroll(&mut late, &c).is_err()
    );
    let paid =
        kingmaker_sim::construction_payroll::execute_construction_payroll(&mut w, &c).unwrap();
    assert!(paid
        .operations
        .iter()
        .any(|o| o.kind == kingmaker_core::operations::OperationKind::Payroll));
}
