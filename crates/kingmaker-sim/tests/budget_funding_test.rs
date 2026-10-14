#[path = "common/budget.rs"]
mod common;
use common::*;
use kingmaker_core::{operations::*, *};
use kingmaker_sim::{budget_cycle::execute_budget_cycle, operation_session::OperationSession};
use serde_json::json;

#[test]
fn authorized_adventurer_issuance_has_exact_tax_and_no_treasury_expense() {
    let mut w = world(0);
    w.budget_data.employment.clear();
    adv(&mut w);
    let r = request(&w);
    let receipt = execute_budget_cycle(&mut w, &r).unwrap();
    assert_eq!(cash(&w, "a", "personal_savings"), 1800);
    assert_eq!(cash(&w, "treasury:s", "operating_capital"), 200);
    assert_eq!(
        receipt.issuance_by_currency,
        vec![("X".into(), Money(2000))]
    );
    let op = receipt
        .operations
        .iter()
        .find(|o| o.kind == OperationKind::AdventurerIncome)
        .unwrap();
    assert!(op.debits.is_empty());
    let base = world(0);
    let mut s = OperationSession::new(&base).unwrap();
    assert!(s.stage(op).is_err());
}
#[test]
fn adventurer_missing_rate_wrong_currency_and_overflow_are_atomic_errors() {
    for kind in 0..3 {
        let mut w = world(1000);
        adv(&mut w);
        match kind {
            0 => w.budget_data.adventurer_income_rates.clear(),
            1 => w.budget_data.adventurer_income_rates[0].currency_id = "Y".into(),
            _ => w.budget_data.adventurer_income_rates[0].rate.numerator = i64::MAX.to_string(),
        };
        let before = w.clone();
        let r = request(&w);
        assert!(execute_budget_cycle(&mut w, &r).is_err());
        assert_eq!(w, before);
    }
}
#[test]
fn finite_treasury_funds_sectors_separately_without_fabricated_cash() {
    let mut w = world(1000);
    let r = request(&w);
    let receipt = execute_budget_cycle(&mut w, &r).unwrap();
    let funded: Vec<_> = receipt
        .operations
        .iter()
        .filter(|o| o.kind == OperationKind::StateBudgetFunding)
        .map(|o| o.credits[0].amount_minor.0)
        .collect();
    assert_eq!(funded, vec![500, 500]);
    assert_eq!(cash(&w, "teacher", "personal_savings"), 450);
    assert_eq!(cash(&w, "medic", "personal_savings"), 450);
    assert_eq!(cash(&w, "treasury:s", "operating_capital"), 100);
    assert_eq!(
        w.production_data
            .obligations
            .iter()
            .map(|d| d.amount_minor.0)
            .sum::<i64>(),
        1000
    );
}
#[test]
fn private_funding_uses_real_owner_money_and_only_excess_wage_cash_for_maintenance() {
    let mut w = world(0);
    w.budget_data.employment.truncate(1);
    let c = &mut w.production_data.contracts[0];
    c.extra.clear();
    c.employer_entity_id = Some("o".into());
    c.payment_purpose = None;
    w.budget_data.foundation_budget_policy = Some(json!({"source":"approved","reserve_weeks":2}));
    w.market_data.maintenance_agreements.push(serde_json::from_value(json!({"id":"m","pop_id":"slave","payer_id":"o","currency_id":"X","weekly_amount_minor":"1000"})).unwrap());
    w.accounts
        .push(account(11, "o", "X", "operating_capital", 500));
    w.accounts
        .push(account(12, "o", "X", "employer_operating_capital", 2500));
    w.rebuild_account_index().unwrap();
    let r = request(&w);
    let receipt = execute_budget_cycle(&mut w, &r).unwrap();
    assert_eq!(
        receipt
            .operations
            .iter()
            .filter(|o| o.kind == OperationKind::BudgetFunding)
            .map(|o| o.debits[0].amount_minor.0)
            .sum::<i64>(),
        1000
    );
    assert_eq!(cash(&w, "o", "employer_operating_capital"), 1000);
    assert_eq!(cash(&w, "o", "sponsored_maintenance_reserve"), 1000);
    assert_eq!(cash(&w, "teacher", "personal_savings"), 900);
}
#[test]
fn forged_direct_funding_and_arbitrary_issuance_are_rejected() {
    let w = world(2000);
    for kind in [
        "adventurer_income",
        "state_budget_funding",
        "budget_funding",
        "budget_phase_boundary",
    ] {
        let op:EconomicOperation=serde_json::from_value(json!({"event_id":"fake","type":kind,"issuance_minor":"20","currency_id":"X","debits":[],"credits":[{"owner_id":"teacher","currency_id":"X","purpose":"personal_savings","amount_minor":"20"}]})).unwrap();
        let mut s = OperationSession::new(&w).unwrap();
        assert!(s.stage(&op).is_err());
    }
}
