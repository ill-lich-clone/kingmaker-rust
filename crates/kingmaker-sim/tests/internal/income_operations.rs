#[path = "../common/production.rs"]
mod common;
use crate::{income_operations::stage_income_operations, operation_session::*};
use kingmaker_core::{operations::*, *};
use serde_json::json;

fn transfer() -> EconomicOperation {
    serde_json::from_value(json!({"event_id":"valid-income","type":"enterprise_income","debits":[{"owner_id":"o","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"5"}],"credits":[{"owner_id":"p","currency_id":"COIN","purpose":"personal_savings","amount_minor":"5"}],"income_recognized":true,"period":"w"})).unwrap()
}
#[test]
fn closed_internal_income_authority_preserves_money_and_prepares_existing_payee_binding() {
    let mut w = common::world();
    let op = transfer();
    let mut session = OperationSession::new(&w).unwrap();
    stage_income_operations(&mut session, &[op]).unwrap();
    let prepared = session.finish().unwrap();
    apply_prepared(&mut w, prepared).unwrap();
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), 20);
    assert_eq!(
        w.accounts
            .iter()
            .find(|a| a.owner_id == ExternalId::from("p"))
            .unwrap()
            .balance,
        Money(5)
    );
}
#[test]
fn unbalanced_income_late_failure_poisons_the_complete_private_session() {
    let w = common::world();
    let op = transfer();
    let mut invalid = op.clone();
    invalid.event_id = "invalid-income".into();
    invalid.credits[0].amount_minor = Money(6);
    let mut session = OperationSession::new(&w).unwrap();
    assert!(stage_income_operations(&mut session, &[op, invalid]).is_err());
    assert!(session.finish().is_err());
}
