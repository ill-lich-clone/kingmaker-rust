#[path = "common/labor.rs"]
mod common;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::{labor::LaborRequest, *};
use kingmaker_sim::profession_cycle::{
    apply_profession_changes, execute_profession_changes, prepare_profession_changes,
};
use serde_json::json;
#[test]
fn full_profession_transfer_publishes_approved_recovery_titles_through_opening() {
    for opening in [false, true] {
        let mut w = fixture(100000);
        w.pops[0].workforce = 1.;
        w.pops[0].size = 2;
        w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["a"] =
            json!([{"workers":1,"weeks":1}]);
        w.labor_data.pop_mobility.as_mut().unwrap()["remainders"] = json!({"a":{"profession":0.9}});
        w.mobility_data.shipments.push(json!({"id":"prior-cargo","buyer_id":"a","status":"interrupted","currency_id":"COIN","goods_price_minor":"0","remaining_quantity":1,"storage_reservation_id":"prior-reservation"}));
        w.transport_data.shipment_recovery_plans.push(json!({"id":"prior-plan","shipment_id":"prior-cargo","accepted_buyer_id":"a","accepted_seller_id":"seller","accepted_carrier_id":"carrier","status":"source","source":"author","author_formula":"keep"}));
        if opening {
            kingmaker_sim::mobility_cycle::execute_mobility_opening(&mut w, &request()).unwrap();
        } else {
            execute_profession_changes(&mut w, &request()).unwrap();
        }
        let buyer = &w.mobility_data.shipments[0]["buyer_id"];
        assert_ne!(
            buyer,
            &json!("a"),
            "must actually transfer the complete workforce"
        );
        assert_eq!(
            &w.transport_data.shipment_recovery_plans[0]["accepted_buyer_id"],
            buyer
        );
        assert_eq!(
            w.transport_data.shipment_recovery_plans[0]["author_formula"],
            json!("keep")
        );
    }
}
fn fixture(cash: i64) -> WorldState {
    let mut w = common::world(cash);
    w.production_data.catalog.as_mut().unwrap().methods[0].staffing = vec!["laborers".into()];
    w.mobility_data.reference = Some(
        json!({"professions":[{"pop_type_id":"laborers","transition_requirements":{"kind":"funded_vacancy","source":"author"}}]}),
    );
    w.labor_data.pop_mobility = Some(json!({"unemployment":{"a":[{"workers":600,"weeks":1}]}}));
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
fn repeated_small_profession_quota_does_not_reject_its_own_rounding_residue() {
    let mut w = fixture(0);
    w.pops[0].workforce = 1.;
    w.pops[0].size = 2;
    w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["a"] =
        json!([{"workers":1,"weeks":1}]);
    for period in 1..=11 {
        let mut r = request();
        r.period = format!("quota-{period}");
        execute_profession_changes(&mut w, &r)
            .expect("Engine-generated quota remains valid next period");
        if period == 10 {
            w = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
            w.rebuild_account_index().unwrap();
        }
    }
    assert!(
        w.labor_data.pop_mobility.as_ref().unwrap()["remainders"]["a"]["profession"]
            .as_f64()
            .unwrap()
            >= 0.
    );
}
#[test]
fn accepted_children_follow_their_parent_in_source_traversal_order() {
    let mut w = fixture(100000);
    w.pops[1].province_id = ProvinceId(1);
    w.pops[1]
        .extra
        .insert("species_labor_coefficient".into(), json!(1));
    w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["b"] =
        json!([{"workers":600,"weeks":1}]);
    let result = execute_profession_changes(&mut w, &request()).unwrap();
    assert_eq!(result.changed_workforce, 120);
    assert_eq!(w.pops[0].external_id, ExternalId::from("a"));
    assert_eq!(w.pops[1].extra["parent_pop_id"], json!("a"));
    assert_eq!(w.pops[2].external_id, ExternalId::from("b"));
    assert_eq!(w.pops[3].extra["parent_pop_id"], json!("b"));
}
#[test]
fn fractional_quota_is_retained_without_forcing_a_person_and_replay_does_not_accrue_twice() {
    for remainder in [0., 0.2] {
        let mut w = fixture(100000);
        w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["a"][0]["workers"] = json!(9);
        w.labor_data.pop_mobility.as_mut().unwrap()["remainders"] =
            json!({"a":{"profession":remainder,"migration":0.7}});
        let receipt = execute_profession_changes(&mut w, &request()).unwrap();
        assert_eq!(
            receipt.changed_workforce,
            if remainder == 0. { 0 } else { 1 }
        );
        let value = w.labor_data.pop_mobility.as_ref().unwrap()["remainders"]["a"]["profession"]
            .as_f64()
            .unwrap();
        assert!((value - if remainder == 0. { 0.9 } else { 0.1 }).abs() < 1e-8);
        assert_eq!(
            w.labor_data.pop_mobility.as_ref().unwrap()["remainders"]["a"]["migration"],
            json!(0.7)
        );
        let before = serde_json::to_value(&w).unwrap();
        execute_profession_changes(&mut w, &request()).unwrap();
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn failure_at_last_source_does_not_publish_the_earlier_successful_child() {
    let mut w = fixture(100000);
    w.pops[1].province_id = ProvinceId(1);
    w.pops[1]
        .extra
        .insert("species_labor_coefficient".into(), json!(1));
    w.pops[1].extra.insert("literate_count".into(), json!(-1));
    w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["b"] =
        json!([{"workers":600,"weeks":1}]);
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_profession_changes(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn orphan_profession_history_or_late_opening_is_rejected_atomically() {
    for late in [false, true] {
        let mut w = fixture(100000);
        if late {
            kingmaker_sim::budget_cycle::execute_budget_cycle(
                &mut w,
                &kingmaker_core::budget::BudgetRequest {
                    expected_revision: 20,
                    period: "w".into(),
                    policies: Default::default(),
                },
            )
            .unwrap();
        } else {
            w.production_data.committed_operations.push(
                kingmaker_core::operations::CommittedOperation {
                    event_id: "profession:[\"w\",\"a\",\"rgo\"]".into(),
                    value: json!({"orphan":true}),
                },
            );
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_profession_changes(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
fn factory_fixture() -> WorldState {
    let mut w = production::world();
    let c = common::world(0);
    w.market_data.needs = c.market_data.needs;
    w.market_data
        .needs
        .as_mut()
        .unwrap()
        .profile_refs
        .truncate(1);
    w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = "p".into();
    w.market_data.price_calibration = c.market_data.price_calibration;
    w.pops[0].pop_type_id = "farmers".into();
    w.pops[0].pop_type = "farmers".into();
    w.pops[0].size = 1000;
    w.pops[0].workforce = 600.;
    w.pops[0].employed = 0.;
    w.pops[0].unemployed = 600.;
    w.pops[0]
        .extra
        .insert("species_labor_coefficient".into(), json!(1));
    w.production_data.factories[0].base.assignments.clear();
    w.production_data.factories[0].staff_count = Some(0.);
    w.production_data.contracts.clear();
    w.accounts[1].balance = Money(100000);
    w.stockpiles[0].quantity = 5.5;
    w.stockpiles[1].quantity = 2.;
    w.mobility_data.reference = Some(
        json!({"professions":[{"pop_type_id":"craftsmen","transition_requirements":{"kind":"funded_vacancy","source":"author"}}]}),
    );
    w.labor_data.pop_mobility = Some(json!({"unemployment":{"p":[{"workers":600,"weeks":1}]}}));
    w
}
#[test]
fn factory_transition_is_limited_by_real_inputs_and_produces_same_period() {
    let mut w = factory_fixture();
    let result = execute_profession_changes(&mut w, &request()).unwrap();
    assert_eq!(result.changed_workforce, 5);
    let production =
        kingmaker_sim::production::execute_production_cycle(&mut w, &production::request())
            .unwrap();
    assert!(production.outputs.iter().any(|r| r.quantity >= 5.));
}
#[test]
fn two_factories_share_raw_inputs_and_warehouse_without_two_promised_outputs() {
    let mut w = factory_fixture();
    let mut second = w.production_data.factories[0].clone();
    second.base.id = "f2".into();
    w.production_data.factories.push(second);
    let mut p = w.pops[0].clone();
    p.id = PopId(2);
    p.external_id = "second-source".into();
    w.pops.push(p);
    let mut r = w.market_data.needs.as_ref().unwrap().profile_refs[0].clone();
    r.pop_id = "second-source".into();
    w.market_data.needs.as_mut().unwrap().profile_refs.push(r);
    w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["second-source"] =
        json!([{"workers":600,"weeks":1}]);
    let result = execute_profession_changes(&mut w, &request()).unwrap();
    assert_eq!(result.changed_workforce, 5);
    assert_eq!(
        w.pops
            .iter()
            .filter(|p| p.extra.contains_key("parent_pop_id"))
            .count(),
        1
    );
    let produced =
        kingmaker_sim::production::execute_production_cycle(&mut w, &production::request())
            .unwrap();
    assert!((produced.outputs.iter().map(|r| r.quantity).sum::<f64>() - 5.).abs() < 1e-8);
    assert!(w.stockpiles.iter().all(|s| s.quantity >= 0.));
}
#[test]
fn service_transition_uses_actual_school_and_treasury_funding() {
    let mut w = fixture(0);
    w.production_data.resource_operations.clear();
    w.labor_data.profiles = vec![json!({"state_id":"s"})];
    w.market_data.institutions = vec![
        json!({"building_id":"school","kind":"school","owner_entity_id":"s","province_id":1,"level":1,"operational_status":"active"}),
    ];
    w.budget_data.jobs = vec![
        json!({"job_id":"teacher","building_id":"school","province_id":1,"pop_type_id":"clergymen","role_id":"teacher","slots_total":2000,"capacity_pool_id":"school","capacity_pool_total":2000}),
    ];
    w.mobility_data.reference = Some(
        json!({"professions":[{"pop_type_id":"clergymen","transition_requirements":{"kind":"funded_vacancy","source":"author"}}]}),
    );
    w.accounts.push(AccountBalance {
        id: AccountId(1000),
        external_id: "treasury".into(),
        owner_id: "treasury:s".into(),
        currency_id: "COIN".into(),
        purpose: "operating_capital".into(),
        balance: Money(100000),
    });
    let result = execute_profession_changes(&mut w, &request()).unwrap();
    assert_eq!(result.changed_workforce, 60);
    assert!(result.operations.iter().any(|op| op
        .debits
        .iter()
        .any(|d| d.owner_id == ExternalId::from("treasury:s"))));
    let child = w
        .pops
        .iter()
        .find(|p| p.extra.contains_key("parent_pop_id"))
        .unwrap();
    assert_eq!(child.pop_type_id, ExternalId::from("clergymen"));
    assert_eq!(child.employed, 60.);
}
#[test]
fn transition_counts_and_own_rates_match_actual_source() {
    for cash in [100000, 1540, 100, 0] {
        let mut w = fixture(cash);
        let mut snapshot = common::snapshot(&w);
        snapshot["registry"]["popMobility"] = w.labor_data.pop_mobility.clone().unwrap();
        snapshot["registry"]["reference"] = w.mobility_data.reference.clone().unwrap();
        let source =
            common::oracle(json!({"action":"profession","snapshot":snapshot,"period":"w"}));
        let actual = execute_profession_changes(&mut w, &request()).unwrap();
        assert_eq!(
            actual.changed_workforce,
            source["result"]["changed_workforce"].as_u64().unwrap(),
            "cash {cash}; {source}"
        );
        if actual.changed_workforce > 0 {
            let native = w
                .production_data
                .contracts
                .iter()
                .find(|c| {
                    c.extra
                        .get("provenance")
                        .and_then(|p| p.get("kind"))
                        .and_then(serde_json::Value::as_str)
                        == Some("own_pop_basket_rate")
                })
                .unwrap();
            assert_eq!(
                native.weekly_wage_rate.as_ref().unwrap().numerator,
                source["registry"]["contracts"][0]["weekly_wage_rate"]["numerator"]
            );
            assert_eq!(
                native.weekly_wage_rate.as_ref().unwrap().denominator,
                source["registry"]["contracts"][0]["weekly_wage_rate"]["denominator"]
            );
        }
    }
}
#[test]
fn aged_farmer_changes_only_to_funded_job_and_child_is_fully_hired() {
    let mut w = fixture(100000);
    let before_people = w.pops.iter().map(|p| p.size).sum::<u32>();
    let before_money = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let date = w.campaign.clone();
    let receipt = execute_profession_changes(&mut w, &request()).unwrap();
    assert_eq!(receipt.changed_workforce, 60);
    let child = w
        .pops
        .iter()
        .find(|p| p.extra.contains_key("parent_pop_id"))
        .unwrap();
    assert_eq!(child.pop_type_id, ExternalId::from("laborers"));
    assert_eq!(child.employed, 60.);
    assert_eq!(child.unemployed, 0.);
    assert_eq!(w.pops.iter().map(|p| p.size).sum::<u32>(), before_people);
    assert_eq!(
        w.accounts.iter().map(|a| a.balance.0).sum::<i64>(),
        before_money
    );
    assert_eq!(w.campaign, date);
    assert_eq!(
        w.labor_data.pop_mobility.as_ref().unwrap()["unemployment"]
            [child.external_id.as_str().unwrap()],
        json!([])
    );
    assert!(w.production_data.resource_operations[0]
        .base
        .assignments
        .iter()
        .any(|a| a.pop_id == child.external_id && a.workers_count == 60.));
}
#[test]
fn small_budget_retries_exact_child_size_and_two_sources_cannot_spend_it_twice() {
    let mut w = fixture(100);
    w.pops[1].province_id = ProvinceId(1);
    w.pops[1]
        .extra
        .insert("species_labor_coefficient".into(), json!(1));
    w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["b"] =
        json!([{"workers":600,"weeks":1}]);
    let before_money = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let receipt = execute_profession_changes(&mut w, &request()).unwrap();
    assert!(
        receipt.changed_workforce > 0 && receipt.changed_workforce < 60,
        "{:?}",
        receipt
    );
    let children: Vec<_> = w
        .pops
        .iter()
        .filter(|p| p.extra.contains_key("parent_pop_id"))
        .collect();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].employed, children[0].workforce);
    assert_eq!(children[0].unemployed, 0.);
    assert_eq!(
        w.accounts.iter().map(|a| a.balance.0).sum::<i64>(),
        before_money
    );
    assert!(w.accounts.iter().all(|a| a.balance.0 >= 0));
}
#[test]
fn missing_requirements_funding_or_age_do_not_force_profession_or_income() {
    for case in 0..4 {
        let mut w = fixture(if case == 0 { 0 } else { 100000 });
        match case {
            1 => w.mobility_data.reference = None,
            2 => {
                w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["a"][0]["weeks"] =
                    json!(0)
            }
            3 => w.pops[0].pop_type_id = "adventurers".into(),
            _ => {}
        }
        let pops = w.pops.clone();
        let accounts = w.accounts.clone();
        let result = execute_profession_changes(&mut w, &request()).unwrap();
        assert_eq!(result.changed_workforce, 0);
        assert_eq!(w.pops, pops);
        assert_eq!(w.accounts, accounts);
    }
}
#[test]
fn replay_stale_read_and_changed_request_cannot_duplicate_or_partially_apply() {
    let mut w = fixture(100000);
    let p = prepare_profession_changes(&w, &request()).unwrap();
    w.accounts[0].balance = Money(9999);
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_profession_changes(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = fixture(100000);
    let result = execute_profession_changes(&mut w, &request()).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        execute_profession_changes(&mut w, &request()).unwrap(),
        result
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut r = request();
    r.policies.wages.insert("s".into(), 0.5);
    assert!(execute_profession_changes(&mut w, &r).is_err());
}
