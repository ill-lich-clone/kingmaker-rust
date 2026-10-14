#[path = "common/labor.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::cohort_scope::CohortScope;
use serde_json::json;
#[test]
fn scoped_split_matches_scalar_money_and_people_with_actual_global_work() {
    let mut w = common::world(0);
    w.production_data.resource_operations[0].base.assignments = serde_json::from_value(
        json!([{"employment_id":"work","pop_id":"a","workers_count":300,"time_fraction":1}]),
    )
    .unwrap();
    let r = kingmaker_core::mobility::CohortSplitRequest {
        expected_revision: 20,
        event_id: "scope:split".into(),
        pop_id: "a".into(),
        workforce_count: 60,
        target: Default::default(),
    };
    let mut scope = CohortScope::new(&w).unwrap();
    let (selected, receipt) = scope.preview(&w, &r).unwrap();
    let mut scalar = w.clone();
    let actual = kingmaker_sim::cohort_cycle::execute_pop_split(&mut scalar, &r).unwrap();
    assert_eq!(receipt, actual);
    let p = selected
        .pops
        .iter()
        .find(|p| p.external_id == ExternalId::from("a"))
        .unwrap();
    assert_eq!(p.employed, 300.);
    assert_eq!(p.unemployed, 240.);
    assert_eq!(w.pops[0].size, 1000);
    assert_eq!(selected.pops.iter().map(|p| p.size).sum::<u32>(), 1000);
}
#[test]
fn owner_slice_keeps_actual_finance_and_rights_without_copying_unrelated_world() {
    let mut w = common::world(0);
    w.mobility_data
        .project_investments
        .push(json!({"id":"mine","investor_id":"a"}));
    w.mobility_data
        .project_investments
        .push(json!({"id":"other","investor_id":"b"}));
    let mut scope = CohortScope::new(&w).unwrap();
    let selected = scope.select(&w, &ExternalId::from("a")).unwrap();
    assert_eq!(selected.pops.len(), 1);
    assert_eq!(selected.accounts.len(), 1);
    assert_eq!(selected.accounts[0], w.accounts[0]);
    assert_eq!(
        selected.mobility_data.project_investments,
        vec![json!({"id":"mine","investor_id":"a"})]
    );
    assert!(selected.source.is_none());
    assert_eq!(w.pops.len(), 2);
}
#[test]
fn newly_split_claim_linking_another_parent_is_visible_in_later_slice() {
    let mut w = common::world(0);
    let mut scope = CohortScope::new(&w).unwrap();
    w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"new-child-claim","payer_id":"b","payee_id":"new-child","currency_id":"COIN","purpose":"personal_savings","amount_minor":"55"})).unwrap());
    let selected = scope.select(&w, &ExternalId::from("b")).unwrap();
    assert_eq!(
        selected.production_data.obligations,
        w.production_data.obligations
    );
}
#[test]
fn warehouse_leases_held_stock_and_numeric_reference_identity_survive_scope() {
    let mut w = common::world(0);
    let mut f = w.production_data.storage_facilities[0].clone();
    f.access_contracts.push(serde_json::from_value(json!({"owner_id":"a","status":"approved","source":"author","approval_ref":"source","price_minor_per_space":"0","currency_id":"COIN","purpose":"personal_savings"})).unwrap());
    f.id = "leased".into();
    w.production_data.storage_facilities.push(f);
    let mut s = w.stockpiles[0].clone();
    s.owner_id = "a".into();
    s.external_id = "personal".into();
    w.stockpiles.push(s);
    w.production_data.goods_reservations.push(
        serde_json::from_value(json!({"stock_id":"personal","quantity":1,"status":"held"}))
            .unwrap(),
    );
    w.mobility_data
        .project_investments
        .push(json!({"investor_id":7}));
    w.mobility_data
        .project_investments
        .push(json!({"investor_id":"7"}));
    let mut scope = CohortScope::new(&w).unwrap();
    let selected = scope.select(&w, &ExternalId::from("a")).unwrap();
    assert_eq!(selected.production_data.storage_facilities.len(), 2);
    assert!(selected
        .production_data
        .storage_facilities
        .iter()
        .any(|f| f.id == ExternalId::from("leased")));
    assert_eq!(selected.production_data.goods_reservations.len(), 1);
    assert_eq!(
        selected.stockpiles.last().unwrap().external_id,
        ExternalId::from("personal")
    );
    assert!(selected.mobility_data.project_investments.is_empty());
}
