#[path = "common/labor.rs"]
mod common;
use kingmaker_core::{labor::LaborRequest, *};
use kingmaker_sim::migration_cycle::{
    apply_migration_departures, execute_migration_arrivals, execute_migration_departures,
    prepare_migration_departures,
};
use serde_json::json;
fn fixture() -> WorldState {
    let mut w = common::world(100000);
    w.production_data.resource_operations[0].base.province_id = ProvinceId(2);
    w.production_data.resource_operations[0].base.facility_id = Some("wh:b".into());
    for p in &mut w.pops {
        p.state_id = Some("S".into());
    }
    for t in &mut w.market_data.territory {
        t.extra.insert("owner_state_id".into(), json!("S"));
        t.extra.insert("population_total".into(), json!(1000));
    }
    w.labor_data.pop_mobility = Some(json!({"unemployment":{"a":[{"workers":600,"weeks":2}]}}));
    w
}
fn request(period: &str) -> LaborRequest {
    LaborRequest {
        expected_revision: 20,
        period: period.into(),
        policies: Default::default(),
    }
}
#[test]
fn in_transit_pop_without_active_journey_rejects_without_writes() {
    for arrivals in [true, false] {
        let mut w = fixture();
        w.pops[0]
            .extra
            .insert("mobility_status".into(), json!("in_transit"));
        let before = serde_json::to_value(&w).unwrap();
        let result = if arrivals {
            execute_migration_arrivals(&mut w, &request("w"))
        } else {
            execute_migration_departures(&mut w, &request("w"))
        };
        assert!(result.is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn departed_pop_with_removed_or_closed_journey_cannot_be_stranded_next_opening() {
    for case in 0..2 {
        let mut w = fixture();
        execute_migration_departures(&mut w, &request("w1")).unwrap();
        if case == 0 {
            w.labor_data.pop_mobility.as_mut().unwrap()["journeys"] = json!([]);
        } else {
            w.labor_data.pop_mobility.as_mut().unwrap()["journeys"][0]["status"] = json!("arrived");
        }
        w.campaign.day = 8;
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_migration_arrivals(&mut w, &request("w2")).is_err());
        assert!(
            kingmaker_sim::mobility_cycle::execute_mobility_opening(&mut w, &request("w2"))
                .is_err()
        );
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn arrival_keeps_currency_claims_stock_location_and_source_need_references() {
    let mut w = fixture();
    let mut stock = w.stockpiles[0].clone();
    stock.id = 1000;
    stock.external_id = "personal-grain".into();
    stock.province_id = ProvinceId(1);
    stock.owner_id = "a".into();
    stock.quantity = 10.;
    stock.purpose = "food_reserve".into();
    stock.facility_id = Some("wh:a".into());
    w.stockpiles.push(stock);
    execute_migration_departures(&mut w, &request("w1")).unwrap();
    let child = w
        .pops
        .iter()
        .find(|p| p.extra.contains_key("parent_pop_id"))
        .unwrap()
        .external_id
        .clone();
    let moved = w
        .stockpiles
        .iter()
        .find(|s| s.owner_id == child)
        .unwrap()
        .clone();
    assert!((moved.quantity - 0.5).abs() < 1e-10);
    let claims = w.production_data.obligations.clone();
    let money = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    w.campaign.day = 8;
    let mut snapshot = common::snapshot(&w);
    snapshot["campaign"] = json!({"date":{"year":306,"month":1,"day":8}});
    snapshot["registry"]["popMobility"] = w.labor_data.pop_mobility.clone().unwrap();
    snapshot["registry"]["territory"] = json!(w.market_data.territory);
    let source = common::oracle(
        json!({"action":"migration_arrival","snapshot":snapshot,"period":"w2","date":{"year":306,"month":1,"day":8}}),
    );
    let result = execute_migration_arrivals(&mut w, &request("w2")).unwrap();
    assert_eq!(
        result.workforce,
        source["result"]["arrived_workforce"].as_u64().unwrap()
    );
    assert_eq!(
        w.stockpiles.iter().find(|s| s.owner_id == child).unwrap(),
        &moved
    );
    assert_eq!(w.production_data.obligations, claims);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), money);
    let reference = w
        .market_data
        .needs
        .as_ref()
        .unwrap()
        .profile_refs
        .iter()
        .find(|r| r.pop_id == child)
        .unwrap();
    assert_eq!(reference.extra["province_id"], json!(2));
    assert_eq!(reference.extra["state_id"], json!("S"));
    assert_eq!(
        w.pops
            .iter()
            .find(|p| p.external_id == child)
            .unwrap()
            .currency_id,
        Some("COIN".into())
    );
    let before = serde_json::to_value(&w).unwrap();
    execute_migration_arrivals(&mut w, &request("w2")).unwrap();
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn departure_ids_paths_people_dates_and_actual_accounts_match_source() {
    for cash in [100000, 0] {
        let mut w = fixture();
        w.accounts
            .iter_mut()
            .find(|a| a.owner_id == ExternalId::from("o"))
            .unwrap()
            .balance = Money(cash);
        let mut snapshot = common::snapshot(&w);
        snapshot["campaign"] = json!({"date":{"year":306,"month":1,"day":1}});
        snapshot["registry"]["territory"] = json!(w.market_data.territory);
        snapshot["registry"]["popMobility"] = w.labor_data.pop_mobility.clone().unwrap();
        let source = common::oracle(
            json!({"action":"migration_departure","snapshot":snapshot,"period":"w"}),
        );
        let result = execute_migration_departures(&mut w, &request("w")).unwrap();
        assert_eq!(
            result.workforce,
            source["result"]["departed_workforce"].as_u64().unwrap()
        );
        if result.workforce > 0 {
            let native = &w.labor_data.pop_mobility.as_ref().unwrap()["journeys"][0];
            let expected = &source["result"]["departures"][0];
            assert_eq!(native, expected);
            let child = w
                .pops
                .iter()
                .find(|p| p.external_id.json() == native["pop_id"])
                .unwrap();
            assert_eq!(child.size, 50);
            assert_eq!(child.workforce, 30.);
        }
    }
}
#[test]
fn missing_traveler_reference_is_rejected_before_new_departures() {
    let mut w = fixture();
    w.labor_data.pop_mobility.as_mut().unwrap()["journeys"] = json!([{"id":"bad","pop_id":"missing","status":"in_transit","destination_offer_id":"rgo","workforce":30,"from_province_id":1,"to_province_id":2,"to_state_id":"S","path":[1,2],"arrival_date":{"year":306,"month":1,"day":8}}]);
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_migration_departures(&mut w, &request("w")).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn internal_walking_split_arrives_next_week_then_can_take_a_real_job() {
    let mut w = fixture();
    let money = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let date = w.campaign.clone();
    assert_eq!(
        execute_migration_arrivals(&mut w, &request("w1"))
            .unwrap()
            .workforce,
        0
    );
    let departed = execute_migration_departures(&mut w, &request("w1")).unwrap();
    assert_eq!(departed.workforce, 30);
    let child = w
        .pops
        .iter()
        .find(|p| p.extra.contains_key("parent_pop_id"))
        .unwrap();
    let childid = child.external_id.clone();
    assert_eq!(child.province_id, ProvinceId(1));
    assert_eq!(child.extra["mobility_status"], json!("in_transit"));
    assert_eq!(child.employed, 0.);
    let journey = &w.labor_data.pop_mobility.as_ref().unwrap()["journeys"][0];
    assert_eq!(
        journey["arrival_date"],
        json!({"year":306,"month":1,"day":8})
    );
    assert_eq!(journey["paid_costs"], json!([]));
    assert_eq!(w.campaign, date);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), money);
    w.campaign.day = 8;
    let arrival = execute_migration_arrivals(&mut w, &request("w2")).unwrap();
    assert_eq!(arrival.workforce, 30);
    let child = w.pops.iter().find(|p| p.external_id == childid).unwrap();
    assert_eq!(child.province_id, ProvinceId(2));
    assert_eq!(child.state_id, Some("S".into()));
    assert_eq!(child.currency_id, Some("COIN".into()));
    assert_eq!(child.extra["mobility_status"], json!("settled"));
    assert_eq!(
        w.market_data.territory[0].extra["population_total"].as_f64(),
        Some(950.)
    );
    assert_eq!(
        w.market_data.territory[1].extra["population_total"].as_f64(),
        Some(1050.)
    );
    let labor = kingmaker_sim::labor_cycle::execute_local_labor(&mut w, &request("w2")).unwrap();
    assert!(labor.hires.iter().any(|h| h["pop_id"] == childid.json()));
    assert!(
        w.pops
            .iter()
            .find(|p| p.external_id == childid)
            .unwrap()
            .employed
            > 0.
    );
}
#[test]
fn foreign_walk_requires_three_weeks_and_directional_permission() {
    for (age, allowed, expected) in [(2, true, 0), (3, false, 0), (3, true, 30)] {
        let mut w = fixture();
        w.labor_data.pop_mobility.as_mut().unwrap()["unemployment"]["a"][0]["weeks"] = json!(age);
        w.market_data.territory[1]
            .extra
            .insert("owner_state_id".into(), json!("T"));
        let mut r = request("w");
        if allowed {
            r.policies.extra.insert(
                "migration_borders".into(),
                json!([{"from_state_id":"S","to_state_id":"T","allowed":true}]),
            );
        }
        assert_eq!(
            execute_migration_departures(&mut w, &r).unwrap().workforce,
            expected
        );
    }
}
#[test]
fn prior_travel_reserves_destination_places_and_no_work_occurs_in_transit() {
    let mut w = fixture();
    let mut traveler = w.pops[0].clone();
    traveler.external_id = "prior-traveler".into();
    traveler.id = PopId(1000);
    traveler.size = 66667;
    traveler.workforce = 40000.;
    traveler.unemployed = 40000.;
    traveler.account_id = None;
    traveler.account_status = "unbound".into();
    traveler
        .extra
        .insert("mobility_status".into(), json!("in_transit"));
    w.pops.push(traveler);
    w.labor_data.pop_mobility.as_mut().unwrap()["journeys"] = json!([{"id":"prior","pop_id":"prior-traveler","status":"in_transit","destination_offer_id":"rgo","workforce":40000,"from_province_id":1,"to_province_id":2,"to_state_id":"S","path":[1,2],"departed_date":{"year":306,"month":1,"day":1},"arrival_date":{"year":306,"month":1,"day":8}}]);
    assert_eq!(
        execute_migration_departures(&mut w, &request("w"))
            .unwrap()
            .workforce,
        0
    );
    let mut w = fixture();
    let result = execute_migration_departures(&mut w, &request("w")).unwrap();
    assert_eq!(result.workforce, 30);
    let child = w
        .pops
        .iter()
        .find(|p| p.extra.contains_key("parent_pop_id"))
        .unwrap()
        .external_id
        .clone();
    let plan =
        kingmaker_sim::local_hiring::plan_local_hiring(&w, "later", &Default::default()).unwrap();
    assert!(plan.hires.iter().all(|h| h["pop_id"] != child.json()));
}
#[test]
fn replay_stale_source_and_invalid_due_journey_do_not_duplicate_or_publish() {
    let mut w = fixture();
    let p = prepare_migration_departures(&w, &request("w")).unwrap();
    w.accounts[0].balance = Money(9999);
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_migration_departures(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = fixture();
    let first = execute_migration_departures(&mut w, &request("w")).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        execute_migration_departures(&mut w, &request("w")).unwrap(),
        first
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    w.campaign.day = 8;
    w.labor_data.pop_mobility.as_mut().unwrap()["journeys"][0]["to_province_id"] = json!(999999);
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_migration_arrivals(&mut w, &request("next")).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
