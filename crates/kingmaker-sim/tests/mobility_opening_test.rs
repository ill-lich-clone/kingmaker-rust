#[path = "common/labor.rs"]
mod common;
use kingmaker_core::{labor::LaborRequest, *};
use kingmaker_sim::mobility_cycle::{
    apply_mobility_opening, execute_mobility_opening, prepare_mobility_opening,
};
use serde_json::json;
#[test]
fn opening_claims_titles_leases_and_required_history_survive_native_roundtrip() {
    let mut w = fixture();
    let mut stock = w.stockpiles[0].clone();
    stock.id = 1000;
    stock.external_id = "owned".into();
    stock.owner_id = "a".into();
    stock.province_id = ProvinceId(1);
    stock.facility_id = Some("wh:a".into());
    stock.quantity = 10.;
    stock.extra.insert("source_note".into(), json!("retain"));
    w.stockpiles.push(stock);
    w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"claim","payer_id":"a","payee_id":"b","currency_id":"COIN","purpose":"personal_savings","amount_minor":"1001"})).unwrap());
    let receipt = execute_mobility_opening(&mut w, &request()).unwrap();
    let value = serde_json::to_value(&w).unwrap();
    let mut restored: WorldState = serde_json::from_value(value.clone()).unwrap();
    restored.rebuild_account_index().unwrap();
    assert_eq!(serde_json::to_value(&restored).unwrap(), value);
    assert_eq!(
        execute_mobility_opening(&mut restored, &request()).unwrap(),
        receipt
    );
    assert_eq!(serde_json::to_value(&restored).unwrap(), value);
    assert!(restored.production_data.storage_facilities.iter().any(|f| f
        .access_contracts
        .iter()
        .any(|c| c.extra.contains_key("cohort_source_owner"))));
    assert!(restored
        .stockpiles
        .iter()
        .filter(|s| s
            .external_id
            .as_str()
            .is_some_and(|id| id.starts_with("owned")))
        .all(|s| s.extra["source_note"] == json!("retain")));
}
fn fixture() -> WorldState {
    let mut w = common::world(100000);
    w.production_data.resource_operations[0].base.province_id = ProvinceId(2);
    for p in &mut w.pops {
        p.state_id = Some("S".into());
        p.extra.insert("species_labor_coefficient".into(), json!(1));
    }
    for t in &mut w.market_data.territory {
        t.extra.insert("owner_state_id".into(), json!("S"));
        t.extra.insert("population_total".into(), json!(1000));
    }
    w.labor_data.pop_mobility = Some(json!({"unemployment":{"a":[{"workers":600,"weeks":2}]}}));
    w
}
fn request() -> LaborRequest {
    LaborRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    }
}
#[test]
fn opening_replay_rejects_missing_required_cohort_receipt() {
    let mut w = fixture();
    execute_mobility_opening(&mut w, &request()).unwrap();
    assert!(!w.mobility_data.cohort_receipts.is_empty());
    w.mobility_data.cohort_receipts.clear();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_mobility_opening(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn opening_replay_rejects_duplicate_conflicting_split_evidence_or_missing_child() {
    for case in 0..3 {
        let mut w = fixture();
        execute_mobility_opening(&mut w, &request()).unwrap();
        let receipt = w.mobility_data.cohort_receipts[0].clone();
        match case {
            0 => w.mobility_data.cohort_receipts.push(receipt),
            1 => w.mobility_data.cohort_receipts[0].request.workforce_count += 1,
            _ => w
                .pops
                .retain(|p| Some(&p.external_id) != receipt.child_id.as_ref()),
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_mobility_opening(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn arrivals_local_jobs_professions_and_departures_form_one_atomic_opening() {
    let mut w = fixture();
    let before_people = w.pops.iter().map(|p| p.size).sum::<u32>();
    let before_cash = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let r = execute_mobility_opening(&mut w, &request()).unwrap();
    assert!(r.local.hired_fte >= 600.);
    assert_eq!(r.departures.workforce, 30);
    assert_eq!(r.professions.changed_workforce, 0);
    assert_eq!(w.pops.iter().map(|p| p.size).sum::<u32>(), before_people);
    assert_eq!(
        w.accounts.iter().map(|a| a.balance.0).sum::<i64>(),
        before_cash
    );
    let child = w
        .pops
        .iter()
        .find(|p| p.extra.get("mobility_status") == Some(&json!("in_transit")))
        .unwrap();
    assert_eq!(child.employed, 0.);
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_mobility_opening(&mut w, &request()).unwrap(), r);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn last_phase_failure_does_not_publish_earlier_job_funding() {
    let mut w = fixture();
    w.pops[0].extra.insert("literate_count".into(), json!(-1));
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_mobility_opening(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn stale_actor_or_policy_and_orphan_subphase_reject_before_publication() {
    let mut w = fixture();
    let p = prepare_mobility_opening(&w, &request()).unwrap();
    w.accounts[0].balance = Money(9999);
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_mobility_opening(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = fixture();
    kingmaker_sim::labor_cycle::execute_local_labor(&mut w, &request()).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_mobility_opening(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
