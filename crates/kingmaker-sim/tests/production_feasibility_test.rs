#[path = "common/production.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::{
    operation_session::*, production_activities::*, production_feasibility::*,
    production_payroll::*,
};
#[test]
fn shared_iron_cash_idle_guarantees_are_fair() {
    let mut w = common::world();
    w.pops[0].workforce = 2.0;
    w.pops[0].size = 2;
    w.stockpiles[1].quantity = 0.8;
    let mut f = w.production_data.factories[0].clone();
    f.base.id = "g".into();
    f.base.assignments[0].employment_id = Some("e2".into());
    w.production_data.factories.push(f);
    for reverse in [false, true] {
        if reverse {
            w.production_data.factories.reverse();
        }
        let mut s = OperationSession::new(&w).unwrap();
        s.configure(&common::request()).unwrap();
        let candidates = plan_production_activities(&w, &common::request()).unwrap();
        let plan = plan_feasible_production(&mut s, &common::request(), &candidates).unwrap();
        assert!(plan
            .activities
            .iter()
            .all(|a| (a.output - 0.5).abs() < 1e-10));
        let paid = pay_production_labor(&mut s, &common::request(), &plan).unwrap();
        assert_eq!(paid.iter().map(|r| r.gross_minor.0).sum::<i64>(), 12);
        assert_eq!(paid.iter().map(|r| r.tax_minor.0).sum::<i64>(), 2);
        assert_eq!(
            s.cash(&"o".into(), &"COIN".into(), "employer_operating_capital"),
            Money(8)
        );
    }
    w.production_data.goods_reservations.push(
        serde_json::from_value(
            serde_json::json!({"stock_id":"iron","quantity":1.1,"status":"held"}),
        )
        .unwrap(),
    );
    let mut s = OperationSession::new(&w).unwrap();
    let a = plan_production_activities(&w, &common::request()).unwrap();
    let p = plan_feasible_production(&mut s, &common::request(), &a).unwrap();
    assert!(p.activities.iter().all(|a| a.output == 0.0));
}
#[test]
fn zero_rate_and_partial_payment_match_original_fte_rule() {
    let mut w = common::world();
    w.production_data.contracts[0]
        .weekly_wage_rate
        .as_mut()
        .unwrap()
        .numerator = "0".into();
    w.accounts[0].balance = Money::ZERO;
    w.production_data.contracts[0].guaranteed_idle_weekly_rate = None;
    let mut s = OperationSession::new(&w).unwrap();
    let a = plan_production_activities(&w, &common::request()).unwrap();
    let p = plan_feasible_production(&mut s, &common::request(), &a).unwrap();
    let r = pay_production_labor(&mut s, &common::request(), &p).unwrap();
    assert_eq!(r[0].gross_minor, Money::ZERO);
    assert_eq!(r[0].actual_fte, 1.0);
    let w = common::world();
    let mut s = OperationSession::new(&w).unwrap();
    let a = plan_production_activities(&w, &common::request()).unwrap();
    let p = plan_feasible_production(&mut s, &common::request(), &a).unwrap();
    s.stage(&kingmaker_core::operations::EconomicOperation::transfer(
        "late".into(),
        AccountAddress {
            owner_id: "o".into(),
            currency_id: "COIN".into(),
            purpose: "employer_operating_capital".into(),
        },
        AccountAddress {
            owner_id: "o".into(),
            currency_id: "COIN".into(),
            purpose: "operating_capital".into(),
        },
        Money(15),
    ))
    .unwrap();
    let r = pay_production_labor(&mut s, &common::request(), &p).unwrap();
    assert_eq!(r[0].actual_fte, 0.5);
    assert_eq!(
        s.debt(&"debt:payroll:fixture:w:e".into())
            .unwrap()
            .amount_minor,
        Money(5)
    );
}
#[test]
fn prior_debt_has_first_claim_and_keeps_original_tax() {
    let mut w = common::world();
    w.production_data.obligations.push(serde_json::from_value(serde_json::json!({"obligation_id":"old","payer_id":"o","payee_id":"p","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"30","tax_rate":0.1,"tax_owner_id":"treasury:s"})).unwrap());
    let mut s = OperationSession::new(&w).unwrap();
    s.configure(&common::request()).unwrap();
    plan_prior_production_obligations(&mut s, "w").unwrap();
    assert_eq!(s.debt(&"old".into()).unwrap().amount_minor, Money(10));
    assert_eq!(
        s.cash(&"p".into(), &"COIN".into(), "personal_savings"),
        Money(18)
    );
    let a = plan_production_activities(&w, &common::request()).unwrap();
    let p = plan_feasible_production(&mut s, &common::request(), &a).unwrap();
    assert_eq!(p.activities[0].output, 0.0);
}
#[test]
fn fully_taxed_debt_creates_zero_payee_account_as_js_does() {
    let mut w = common::world();
    w.production_data.obligations.push(serde_json::from_value(serde_json::json!({"obligation_id":"old","payer_id":"o","payee_id":"p","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"10","tax_rate":1.0,"tax_owner_id":"treasury:s"})).unwrap());
    let mut s = OperationSession::new(&w).unwrap();
    plan_prior_production_obligations(&mut s, "w").unwrap();
    let address = AccountAddress {
        owner_id: "p".into(),
        currency_id: "COIN".into(),
        purpose: "personal_savings".into(),
    };
    assert_eq!(s.account(&address).unwrap().balance, Money::ZERO);
}
#[test]
fn old_debt_groups_use_opening_cash_and_not_each_others_new_credit() {
    let mut w = common::world();
    for row in [
        serde_json::json!({"obligation_id":"first","payer_id":"o","payee_id":"o","payee_purpose":"operating_capital","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"10","tax_rate":0}),
        serde_json::json!({"obligation_id":"second","payer_id":"o","payee_id":"p","currency_id":"COIN","purpose":"operating_capital","amount_minor":"5","tax_rate":0}),
    ] {
        w.production_data
            .obligations
            .push(serde_json::from_value(row).unwrap());
    }
    let mut s = OperationSession::new(&w).unwrap();
    plan_prior_production_obligations(&mut s, "w").unwrap();
    assert_eq!(s.debt(&"second".into()).unwrap().amount_minor, Money(5));
    assert_eq!(
        s.cash(&"o".into(), &"COIN".into(), "operating_capital"),
        Money(10)
    );
}
