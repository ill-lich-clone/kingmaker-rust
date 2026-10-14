#[path = "common/production.rs"]
mod common;
use kingmaker_core::operations::*;
use kingmaker_core::*;
use kingmaker_sim::operation_session::*;
use serde_json::json;
#[test]
fn public_income_or_retooling_payload_cannot_grant_payment_authority() {
    for kind in [
        "enterprise_income",
        "enterprise_dividend",
        "artisan_retooling",
        "artisan_retooling_complete",
    ] {
        let w = common::world();
        let op:EconomicOperation=serde_json::from_value(json!({"event_id":"forged-income","type":kind,"debits":[{"owner_id":"o","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"5"}],"credits":[{"owner_id":"p","currency_id":"COIN","purpose":"personal_savings","amount_minor":"5"}],"income_recognized":true,"period":"w"})).unwrap();
        let mut session = OperationSession::new(&w).unwrap();
        assert!(session.stage(&op).is_err());
        assert!(session.finish().is_err());
    }
}
fn address(owner: &str, purpose: &str) -> AccountAddress {
    AccountAddress {
        owner_id: owner.into(),
        currency_id: "COIN".into(),
        purpose: purpose.into(),
    }
}
#[test]
fn public_cohort_payload_cannot_grant_new_accounts_or_claims() {
    let w = common::world();
    for kind in ["pop_cohort_split", "pop_cohort_claim"] {
        let op: EconomicOperation = serde_json::from_value(
            json!({"event_id":"fake-child","type":kind,"obligation":null,"debits":[],"credits":[]}),
        )
        .unwrap();
        let mut session = OperationSession::new(&w).unwrap();
        assert!(session.stage(&op).is_err());
        assert!(session.finish().is_err());
    }
}
fn payroll() -> EconomicOperation {
    serde_json::from_value(json!({"event_id":"payroll:fixture:w:e","type":"payroll","period":"w","employment_id":"e","pop_id":"p","employer_id":"o","currency_id":"COIN","gross_minor":"10","paid_gross_minor":"10","tax_minor":"1","net_minor":"9","tax_rate":0.1,"tax_owner_id":"treasury:s","planned_fte":1,"actual_fte":1,"idle_guarantee_minor":"0",
        "debits":[{"owner_id":"o","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"10"}],"credits":[{"owner_id":"p","currency_id":"COIN","purpose":"personal_savings","amount_minor":"9"},{"owner_id":"treasury:s","currency_id":"COIN","purpose":"operating_capital","amount_minor":"1"}]})).unwrap()
}
#[test]
fn lawful_payroll_credit_creates_only_derived_account() {
    let mut w = common::world();
    let mut s = OperationSession::new(&w).unwrap();
    s.stage(&payroll()).unwrap();
    let changes = s.finish().unwrap();
    apply_prepared(&mut w, changes).unwrap();
    assert_eq!(
        w.account_by_address(&address("p", "personal_savings"))
            .unwrap()
            .balance,
        Money(9)
    );
    assert_eq!(
        w.account_by_address(&address("treasury:s", "operating_capital"))
            .unwrap()
            .balance,
        Money(1)
    );
    assert_eq!(
        w.account_by_address(&address("p", "personal_savings"))
            .unwrap()
            .external_id
            .json(),
        json!("[\"p\",\"COIN\",\"personal_savings\"]")
    );
    assert!(w.pops[0].account_id.is_some());
}
#[test]
fn committed_js_bigint_shape_replays_without_new_cash() {
    let mut w = common::world();
    let op = payroll();
    let mut source = serde_json::to_value(&op).unwrap();
    source["gross_minor"] = json!({"__kingmaker_codec_v2__":"bigint","value":"10"});
    source["paid_gross_minor"] = json!({"__kingmaker_codec_v2__":"bigint","value":"10"});
    source.as_object_mut().unwrap().remove("stock_changes");
    source.as_object_mut().unwrap().remove("obligation");
    w.production_data
        .committed_operations
        .push(CommittedOperation {
            event_id: op.event_id.clone(),
            value: source,
        });
    w.accounts[0].balance = Money::ZERO;
    let mut s = OperationSession::new(&w).unwrap();
    assert_eq!(s.stage(&op).unwrap(), ApplyOutcome::AlreadyApplied);
}
#[test]
fn prepared_payroll_rejects_changed_worker_or_contract() {
    for change_pop in [true, false] {
        let mut w = common::world();
        let mut s = OperationSession::new(&w).unwrap();
        s.stage(&payroll()).unwrap();
        let d = s.finish().unwrap();
        if change_pop {
            w.pops[0].state_id = Some("other".into());
        } else {
            w.production_data.contracts[0]
                .weekly_wage_rate
                .as_mut()
                .unwrap()
                .numerator = "99".into();
        }
        let before = w.clone();
        assert!(apply_prepared(&mut w, d).is_err());
        assert_eq!(w, before);
    }
}
#[test]
fn arbitrary_new_payee_is_rejected_even_with_balanced_legs() {
    let w = common::world();
    let mut op = payroll();
    op.credits[0].owner_id = "stranger".into();
    let mut s = OperationSession::new(&w).unwrap();
    assert!(s.stage(&op).is_err());
    assert!(s.finish().is_err());
    let mut s = OperationSession::new(&w).unwrap();
    let op = EconomicOperation::transfer(
        "unknown".into(),
        address("o", "employer_operating_capital"),
        address("stranger", "personal_savings"),
        Money(1),
    );
    assert!(s.stage(&op).is_err());
}
#[test]
fn mixed_operation_is_atomic_and_failure_poisons_phase() {
    let mut w = common::world();
    w.accounts
        .push(common::account(3, "rich", "operating_capital", i64::MAX));
    w.rebuild_account_index().unwrap();
    let before = w.clone();
    let mut s = OperationSession::new(&w).unwrap();
    s.stage(&EconomicOperation::transfer(
        "move".into(),
        address("o", "employer_operating_capital"),
        address("o", "operating_capital"),
        Money(5),
    ))
    .unwrap();
    assert!(s
        .stage(&EconomicOperation::transfer(
            "overflow".into(),
            address("o", "employer_operating_capital"),
            address("rich", "operating_capital"),
            Money(1)
        ))
        .is_err());
    assert!(s.finish().is_err());
    assert_eq!(w, before);
}
#[test]
fn replay_precedes_current_funds_check_and_conflicting_payload_rejects() {
    let mut w = common::world();
    let op = EconomicOperation::transfer(
        "once".into(),
        address("o", "employer_operating_capital"),
        address("o", "operating_capital"),
        Money(20),
    );
    let mut s = OperationSession::new(&w).unwrap();
    s.stage(&op).unwrap();
    let d = s.finish().unwrap();
    apply_prepared(&mut w, d).unwrap();
    let before = w.clone();
    let mut s = OperationSession::new(&w).unwrap();
    assert_eq!(s.stage(&op).unwrap(), ApplyOutcome::AlreadyApplied);
    let d = s.finish().unwrap();
    apply_prepared(&mut w, d).unwrap();
    assert_eq!(w, before);
    let mut altered = op.clone();
    altered.debits[0].amount_minor = Money(1);
    altered.credits[0].amount_minor = Money(1);
    let mut s = OperationSession::new(&w).unwrap();
    assert!(s.stage(&altered).is_err());
    assert_eq!(w, before);
}
#[test]
fn prepared_changes_reject_changed_rows_and_currencies_never_offset() {
    let mut w = common::world();
    let mut s = OperationSession::new(&w).unwrap();
    s.stage(&payroll()).unwrap();
    let d = s.finish().unwrap();
    w.accounts[0].balance = Money(19);
    let before = w.clone();
    assert!(apply_prepared(&mut w, d).is_err());
    assert_eq!(w, before);
    let mut op = payroll();
    op.credits[0].currency_id = "OTHER".into();
    let mut s = OperationSession::new(&w).unwrap();
    assert!(s.stage(&op).is_err());
}
#[test]
fn invalid_final_stock_change_does_not_publish_earlier_payroll() {
    let w = common::world();
    let before = w.clone();
    let mut s = OperationSession::new(&w).unwrap();
    s.stage(&payroll()).unwrap();
    let op: EconomicOperation=serde_json::from_value(json!({"event_id":"production:fixture:w:act","type":"production","method_kind":"factory","subject_id":"f","activity_id":"act","goods_id":"steel","output_quantity":1,"stock_changes":[{"stock_id":"iron","delta":-2},{"stock_id":"coal","delta":-0.4},{"stock_id":"out","delta":1,"create":{"owner_id":"o","province_id":1,"goods_id":"steel","unit":"t","facility_id":"wh","currency_id":"COIN","inventory_origin":"produced","method_kind":"factory"}}]})).unwrap();
    assert!(s.stage(&op).is_err());
    assert!(s.finish().is_err());
    assert_eq!(w, before);
}
#[test]
fn commit_preflight_failure_cannot_leave_money_published() {
    let mut w = common::world();
    let mut s = OperationSession::new(&w).unwrap();
    s.stage(&payroll()).unwrap();
    let changes = s.finish().unwrap();
    w.production_data.factories[0]
        .base
        .extra
        .insert("level".into(), json!("broken"));
    let before = w.clone();
    assert!(apply_prepared(&mut w, changes).is_err());
    assert_eq!(
        w, before,
        "Derived cache failure cannot follow money writes"
    );
    let mut w = common::world();
    let mut s = OperationSession::new(&w).unwrap();
    s.stage(&payroll()).unwrap();
    let changes = s.finish().unwrap();
    w.accounts.push(w.accounts[1].clone());
    let before = w.clone();
    assert!(apply_prepared(&mut w, changes).is_err());
    assert_eq!(
        w, before,
        "Duplicate unrelated account must reject before writes"
    );
}
#[test]
fn typed_operation_headers_cannot_be_shadowed_by_extra_metadata() {
    let w = common::world();
    let mut op = payroll();
    op.meta.insert("event_id".into(), json!("different"));
    let mut s = OperationSession::new(&w).unwrap();
    assert!(s.stage(&op).is_err());
}
#[test]
fn fake_zero_payroll_does_not_prove_paid_work() {
    let w = common::world();
    let mut op = payroll();
    op.debits.clear();
    op.credits.clear();
    for key in ["gross_minor", "paid_gross_minor", "tax_minor", "net_minor"] {
        op.meta.insert(key.into(), json!("0"));
    }
    let mut s = OperationSession::new(&w).unwrap();
    assert!(
        s.stage(&op).is_err(),
        "Nonzero authored working wage cannot become a free receipt"
    );
}
#[test]
fn payroll_cannot_invent_authored_gross_or_tax_policy() {
    let w = common::world();
    let mut op = payroll();
    op.meta.insert("gross_minor".into(), json!("20"));
    op.meta.insert("paid_gross_minor".into(), json!("20"));
    op.meta.insert("net_minor".into(), json!("18"));
    op.meta.insert("tax_minor".into(), json!("2"));
    op.debits[0].amount_minor = Money(20);
    op.credits[0].amount_minor = Money(18);
    op.credits[1].amount_minor = Money(2);
    let mut s = OperationSession::new(&w).unwrap();
    assert!(s.stage(&op).is_err());
    let mut op = payroll();
    op.meta.insert("tax_rate".into(), json!(0.0));
    op.meta.insert("tax_minor".into(), json!("0"));
    op.meta.insert("net_minor".into(), json!("10"));
    op.meta.insert("tax_owner_id".into(), json!(null));
    op.credits.pop();
    op.credits[0].amount_minor = Money(10);
    let mut s = OperationSession::new(&w).unwrap();
    assert!(s.stage(&op).is_err());
}
