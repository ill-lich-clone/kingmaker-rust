#[path = "common/labor.rs"]
mod common;
use kingmaker_core::{labor::LaborRequest, *};
use kingmaker_sim::{
    mobility_cycle::execute_mobility_opening, mobility_invariants::check_mobility_operations,
};
use serde_json::json;
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
fn altered_profession_or_wage_claim_is_not_hidden_by_money_conservation() {
    let mut before = fixture();
    before.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"wage","payer_id":"a","payee_id":"b","currency_id":"COIN","purpose":"personal_savings","amount_minor":"1001"})).unwrap());
    let mut good = before.clone();
    let receipt = execute_mobility_opening(&mut good, &request()).unwrap();
    assert!(check_mobility_operations(&before, &good, &receipt).all_required_passed());
    for case in 0..2 {
        let mut bad = good.clone();
        if case == 0 {
            bad.pops
                .iter_mut()
                .find(|p| p.extra.contains_key("parent_pop_id"))
                .unwrap()
                .pop_type_id = "capitalists".into();
        } else {
            bad.production_data.obligations[0].amount_minor = Money(9999);
        }
        assert!(!check_mobility_operations(&before, &bad, &receipt).all_required_passed());
    }
}
#[test]
fn lawful_splits_and_new_accounts_are_explained_without_requiring_fixed_pop_ids() {
    let mut w = fixture();
    let mut s = w.stockpiles[0].clone();
    s.id = 1000;
    s.owner_id = "a".into();
    s.province_id = ProvinceId(1);
    s.external_id = "personal".into();
    s.quantity = 10.;
    w.stockpiles.push(s);
    let before = w.clone();
    let receipt = execute_mobility_opening(&mut w, &request()).unwrap();
    let report = check_mobility_operations(&before, &w, &receipt);
    assert!(report.all_required_passed(), "{:?}", report);
}
#[test]
fn altered_child_people_money_titles_and_worker_time_are_not_hidden_by_equal_totals() {
    let before = fixture();
    let mut good = before.clone();
    let receipt = execute_mobility_opening(&mut good, &request()).unwrap();
    for case in 0..4 {
        let mut bad = good.clone();
        let child = bad
            .pops
            .iter()
            .position(|p| p.extra.contains_key("parent_pop_id"))
            .unwrap();
        match case {
            0 => {
                bad.pops[child].size += 1;
                bad.pops[0].size -= 1;
            }
            1 => bad.accounts[0].balance.0 += 1,
            2 => bad.accounts[0].owner_id = "thief".into(),
            _ => bad.pops[child].employed = 1.,
        }
        assert!(!check_mobility_operations(&before, &bad, &receipt).all_required_passed());
    }
}
#[test]
fn missing_cohort_or_committed_operation_history_is_rejected() {
    let before = fixture();
    let mut good = before.clone();
    let receipt = execute_mobility_opening(&mut good, &request()).unwrap();
    for history in [false, true] {
        let mut bad = good.clone();
        if history {
            bad.production_data.committed_operations.pop();
        } else {
            bad.mobility_data.cohort_receipts.clear();
        }
        assert!(!check_mobility_operations(&before, &bad, &receipt).all_required_passed());
    }
}
