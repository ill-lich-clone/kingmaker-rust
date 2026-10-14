mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::{market::*, production::ProductionRequest};
use kingmaker_sim::{market_cycle::*, operation_session::*, production::execute_production_cycle};
use serde_json::json;
fn request(stage: MarketStage) -> MarketRequest {
    MarketRequest {
        expected_revision: 20,
        period: "w".into(),
        stage,
        profile: Default::default(),
        priorities: None,
    }
}
#[test]
fn maintenance_unfilled_second_need_limits_work_and_forged_receipt_rejects() {
    let mut w = market_world();
    w.pops[1].pop_type_id = "slaves".into();
    w.market_data.maintenance_agreements =
        serde_json::from_value(json!([{"pop_id":"b","payer_id":"a","currency_id":"COIN"}]))
            .unwrap();
    w.accounts[0].purpose = "sponsored_maintenance_reserve".into();
    w.market_data.needs.as_mut().unwrap().profiles[0].norms.push(serde_json::from_value(json!({"need_id":"clothing","goods_id":"clothing","H":1,"allocation_rule":"none","quantity_per_person_week":0.001,"budget_scope":"basic"})).unwrap());
    let r = execute_market_phase(&mut w, &request(MarketStage::Maintenance)).unwrap();
    assert_eq!(r.maintenance_receipts.len(), 1);
    let receipt = &r.maintenance_receipts[0];
    assert_eq!(receipt.fraction, 0.0);
    assert!(receipt.extra["demands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["need_id"] == "clothing" && r["actual_quantity"].as_f64() == Some(0.0)));
    let mut forged = receipt.clone();
    forged.fraction = 1.0;
    let p = ProductionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
        profile: Default::default(),
        maintenance_receipts: vec![],
    };
    let s = OperationSession::new(&w).unwrap();
    assert!(!kingmaker_sim::production_validation::maintenance_verified(
        &s, &p, &forged
    ));
}
#[test]
fn replay_is_identical_conflict_and_partial_history_reject_atomically() {
    let mut w = market_world();
    let req = request(MarketStage::EarlyFood);
    let r = execute_market_phase(&mut w, &req).unwrap();
    let before = w.clone();
    assert_eq!(execute_market_phase(&mut w, &req).unwrap(), r);
    assert_eq!(w, before);
    let mut different = req.clone();
    different.profile.food_norm_multiplier = Some(2.0);
    assert!(execute_market_phase(&mut w, &different).is_err());
    assert_eq!(w, before);
    w.market_data.phase_receipts.clear();
    let before = w.clone();
    assert!(execute_market_phase(&mut w, &req).is_err());
    assert_eq!(w, before);
}
#[test]
fn read_change_after_prepare_cannot_publish_earlier_cash() {
    let mut w = market_world();
    let prepared = prepare_market_phase(&w, &request(MarketStage::EarlyFood)).unwrap();
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("flour".into(), json!(2));
    let before = w.clone();
    assert!(apply_market_phase(&mut w, prepared).is_err());
    assert_eq!(w, before);
}
#[test]
fn closing_requires_same_period_native_production_and_does_not_double_eat() {
    let mut w = market_world();
    let req = request(MarketStage::ClosingMarket);
    let before = w.clone();
    assert!(execute_market_phase(&mut w, &req).is_err());
    assert_eq!(w, before);
    let early = execute_market_phase(&mut w, &request(MarketStage::EarlyFood)).unwrap();
    let p = ProductionRequest {
        expected_revision: 20,
        period: "wrong".into(),
        policies: Default::default(),
        profile: Default::default(),
        maintenance_receipts: vec![],
    };
    execute_production_cycle(&mut w, &p).unwrap();
    let before = w.clone();
    assert!(execute_market_phase(&mut w, &req).is_err());
    assert_eq!(w, before);
    let mut p = p;
    p.period = "w".into();
    execute_production_cycle(&mut w, &p).unwrap();
    let closing = execute_market_phase(&mut w, &req).unwrap();
    let eaten = early
        .operations
        .iter()
        .chain(closing.operations.iter())
        .filter(|o| {
            o.kind == kingmaker_core::operations::OperationKind::Purchase
                && o.meta.get("consumed") == Some(&json!(true))
                && o.meta.get("need_id") == Some(&json!("food"))
        })
        .map(|o| o.meta["quantity"].as_f64().unwrap())
        .sum::<f64>();
    assert_close(eaten, 20.0 / 1.03, 1e-9);
    assert_eq!(w.campaign.revision, 20);
    assert_eq!(
        (w.campaign.year, w.campaign.month, w.campaign.day),
        (306, 1, 1)
    );
    assert!(closing.invariants.production_required_passed());
    let closing_eaten = closing
        .consumption
        .iter()
        .filter(|r| r["need_id"] == "food")
        .map(|r| r["actual_quantity"].as_f64().unwrap())
        .sum::<f64>();
    assert_close(closing_eaten, eaten, 1e-9);
}

#[test]
fn full_maintenance_receipt_is_verified_and_changed_norm_or_duplicate_event_rejects() {
    let mut w = market_world();
    w.pops[1].pop_type_id = "slaves".into();
    w.market_data.maintenance_agreements =
        serde_json::from_value(json!([{"pop_id":"b","payer_id":"a","currency_id":"COIN"}]))
            .unwrap();
    w.accounts[0].purpose = "sponsored_maintenance_reserve".into();
    let r = execute_market_phase(&mut w, &request(MarketStage::Maintenance)).unwrap();
    let receipt = &r.maintenance_receipts[0];
    assert_eq!(receipt.fraction, 1.0);
    let p = ProductionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
        profile: Default::default(),
        maintenance_receipts: vec![],
    };
    let s = OperationSession::new(&w).unwrap();
    assert!(kingmaker_sim::production_validation::maintenance_verified(
        &s, &p, receipt
    ));
    let mut forged = receipt.clone();
    forged.event_ids.push(forged.event_ids[0].clone());
    assert!(!kingmaker_sim::production_validation::maintenance_verified(
        &s, &p, &forged
    ));
    w.market_data.needs.as_mut().unwrap().profiles[0].norms[0]
        .extra
        .insert("quantity_per_person_week".into(), json!(0.02));
    let s = OperationSession::new(&w).unwrap();
    assert!(!kingmaker_sim::production_validation::maintenance_verified(
        &s, &p, receipt
    ));
}

#[test]
fn first_early_or_maintenance_after_production_or_closing_rejects_without_second_meal() {
    let mut w = market_world();
    w.stockpiles[0].quantity = 80.0;
    let p = ProductionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
        profile: Default::default(),
        maintenance_receipts: vec![],
    };
    execute_production_cycle(&mut w, &p).unwrap();
    let before = w.clone();
    assert!(execute_market_phase(&mut w, &request(MarketStage::EarlyFood)).is_err());
    assert_eq!(w, before);
    execute_market_phase(&mut w, &request(MarketStage::ClosingMarket)).unwrap();
    let before = w.clone();
    assert!(execute_market_phase(&mut w, &request(MarketStage::EarlyFood)).is_err());
    assert!(execute_market_phase(&mut w, &request(MarketStage::Maintenance)).is_err());
    assert_eq!(w, before);
}
#[test]
fn closing_inherits_confirmed_production_priority_override_unless_explicitly_overridden() {
    let mut w = market_world();
    w.production_data.policies.priorities = vec!["basic".into(), "everyday".into()];
    let p = ProductionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: serde_json::from_value(json!({"priorities":["everyday","basic"]})).unwrap(),
        profile: Default::default(),
        maintenance_receipts: vec![],
    };
    let prod = execute_production_cycle(&mut w, &p).unwrap();
    let r = execute_market_phase(&mut w, &request(MarketStage::ClosingMarket)).unwrap();
    assert_eq!(r.effective_priorities, prod.purchase_priorities);
    let mut w = market_world();
    w.production_data.policies.priorities = vec!["basic".into(), "everyday".into()];
    execute_production_cycle(&mut w, &p).unwrap();
    let mut req = request(MarketStage::ClosingMarket);
    req.priorities = Some(vec![]);
    assert!(execute_market_phase(&mut w, &req)
        .unwrap()
        .effective_priorities
        .is_empty());
}
