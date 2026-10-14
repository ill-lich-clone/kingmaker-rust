#[path = "common/native_week.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::{native_week::execute_native_week, week_history::validate_week_history};
use serde_json::json;
#[test]
fn empty_complete_week_has_required_history_and_replay_survives_native_snapshot() {
    let mut w = common::empty();
    let r = common::request(&w);
    let receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    assert_eq!(receipt.end_revision, 21);
    assert_eq!(w.campaign.day, 8);
    assert_eq!(receipt.phase_order.first().unwrap(), "storage_capacity");
    assert!(!receipt.operation_evidence.is_empty());
    assert!(!receipt.registry_evidence.is_empty());
    assert_eq!(
        validate_week_history(&w, &r).unwrap(),
        Some(receipt.clone())
    );
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        execute_native_week(&mut w, &r, |_| panic!("replay must not run phases")).unwrap(),
        receipt
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut loaded: WorldState = serde_json::from_value(before).unwrap();
    loaded.rebuild_account_index().unwrap();
    assert_eq!(
        execute_native_week(&mut loaded, &r, |_| {}).unwrap(),
        receipt
    );
}
#[test]
fn missing_or_tampered_week_and_required_phase_evidence_refuse_without_mutation() {
    let mut w = common::empty();
    let r = common::request(&w);
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    for case in 0..6 {
        let mut bad = w.clone();
        match case {
            0 => bad.week_data.phase_receipts.clear(),
            1 => bad.week_data.phase_receipts[0].end_revision += 1,
            2 => bad.week_data.phase_receipts[0]
                .phase_order
                .pop()
                .map(|_| ())
                .unwrap(),
            3 => bad.market_data.phase_receipts.clear(),
            4 => {
                bad.production_data.committed_operations.pop();
            }
            _ => bad.week_data.phase_receipts[0]
                .diagnostics
                .push(json!({"fake":true})),
        };
        let before = serde_json::to_value(&bad).unwrap();
        assert!(validate_week_history(&bad, &r).is_err(), "case {case}");
        assert_eq!(serde_json::to_value(&bad).unwrap(), before);
    }
}
#[test]
fn whole_week_retains_distinct_numeric_and_string_owners_and_currency_ledgers() {
    let mut w = common::empty();
    for (id, owner, currency, cash) in [(1, json!(7), json!(5), 9), (2, json!("7"), json!("5"), 10)]
    {
        w.accounts.push(AccountBalance {
            id: AccountId(id),
            external_id: format!("cash:{id}").into(),
            owner_id: ExternalId::from_json(owner).unwrap(),
            currency_id: ExternalId::from_json(currency).unwrap(),
            purpose: "personal_savings".into(),
            balance: Money(cash),
        });
    }
    w.rebuild_account_index().unwrap();
    let before = w.accounts.clone();
    let r = common::request(&w);
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    assert_eq!(w.accounts, before);
    let mut loaded: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    loaded.rebuild_account_index().unwrap();
    assert_eq!(loaded.accounts, before);
    assert!(validate_week_history(&loaded, &r).unwrap().is_some());
}

#[test]
fn missing_changed_or_duplicate_mobility_subphase_history_refuses_week_replay() {
    let mut w = common::empty();
    let r = common::request(&w);
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    for case in 0..5 {
        let mut bad = w.clone();
        match case {
            0 => bad.mobility_data.profession_receipts.clear(),
            1 => bad.mobility_data.migration_receipts.clear(),
            2 => bad.mobility_data.profession_receipts[0].changed_workforce += 1,
            3 => bad
                .mobility_data
                .profession_receipts
                .push(bad.mobility_data.profession_receipts[0].clone()),
            _ => bad
                .mobility_data
                .migration_receipts
                .push(bad.mobility_data.migration_receipts[0].clone()),
        }
        let before = serde_json::to_value(&bad).unwrap();
        assert!(validate_week_history(&bad, &r).is_err(), "case {case}");
        assert_eq!(serde_json::to_value(&bad).unwrap(), before);
    }
}

#[test]
fn a_fresh_period_cannot_bypass_a_rolled_back_or_advanced_completed_week_header() {
    let mut w = common::empty();
    let r = common::request(&w);
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    for case in 0..4 {
        let mut bad = w.clone();
        match case {
            0 => {
                bad.campaign.revision = 20;
                bad.campaign.day = 1;
            }
            1 => bad.campaign.revision = 20,
            2 => bad.campaign.day = 1,
            _ => bad.campaign.day += 7,
        }
        let mut next = common::request(&bad);
        next.period = "fresh-period".into();
        let before = serde_json::to_value(&bad).unwrap();
        assert!(
            execute_native_week(&mut bad, &next, |_| panic!(
                "invalid header must refuse before phases"
            ))
            .is_err(),
            "case {case}"
        );
        assert_eq!(serde_json::to_value(&bad).unwrap(), before);
    }
}
