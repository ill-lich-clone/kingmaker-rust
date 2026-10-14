#[path = "common/labor.rs"]
mod common;
use kingmaker_core::{production::ProductionPolicies, *};
use kingmaker_sim::local_hiring::{plan_local_hiring, plan_local_hiring_scoped, HiringScope};
use std::collections::HashSet;
#[test]
fn scope_hires_only_requested_pop_and_offer_without_removing_other_work() {
    let mut w = common::world(100000);
    w.pops[1].province_id = ProvinceId(1);
    w.pops[1]
        .extra
        .insert("species_labor_coefficient".into(), serde_json::json!(1));
    let mut other = w.production_data.resource_operations[0].clone();
    other.base.id = "other-rgo".into();
    w.production_data.resource_operations.push(other);
    let scope = HiringScope {
        candidate_ids: Some(HashSet::from([ExternalId::from("b")])),
        offer_ids: Some(HashSet::from([ExternalId::from("other-rgo")])),
        ..Default::default()
    };
    let before = serde_json::to_value(&w).unwrap();
    let result = plan_local_hiring_scoped(&w, "w", &ProductionPolicies::default(), &scope).unwrap();
    assert!(result.hired_fte() > 0.);
    assert!(result
        .hires
        .iter()
        .all(|h| h["pop_id"] == serde_json::json!("b")
            && h["entity_id"] == serde_json::json!("other-rgo")));
    assert_eq!(result.pops()[0].employed, 0.);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn external_wage_reserve_protects_money_for_workers_outside_the_slice() {
    let w = common::world(1540);
    assert!(
        plan_local_hiring(&w, "w", &ProductionPolicies::default())
            .unwrap()
            .hired_fte()
            > 0.
    );
    let scope = HiringScope {
        external_wage_reservations: vec![(
            AccountAddress {
                owner_id: "o".into(),
                currency_id: "COIN".into(),
                purpose: "operating_capital".into(),
            },
            Money(1540),
        )],
        ..Default::default()
    };
    let result = plan_local_hiring_scoped(&w, "w", &ProductionPolicies::default(), &scope).unwrap();
    assert_eq!(result.hired_fte(), 0.);
    assert!(result.operations.is_empty());
}
#[test]
fn absent_filters_preserve_unfiltered_contracts_people_and_allocation() {
    let w = common::world(1540);
    let full = plan_local_hiring(&w, "w", &ProductionPolicies::default()).unwrap();
    let scoped = plan_local_hiring_scoped(
        &w,
        "w",
        &ProductionPolicies::default(),
        &HiringScope::default(),
    )
    .unwrap();
    assert_eq!(scoped.hired_fte(), full.hired_fte());
    assert_eq!(scoped.contracts(), full.contracts());
    assert_eq!(scoped.employment(), full.employment());
    assert_eq!(scoped.pops(), full.pops());
    assert_eq!(scoped.operations, full.operations);
}
