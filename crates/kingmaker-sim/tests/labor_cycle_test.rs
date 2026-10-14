#[path = "common/budget.rs"]
mod budget;
#[path = "common/labor.rs"]
mod common;
use kingmaker_core::{labor::LaborRequest, *};
use kingmaker_sim::{budget_cycle::execute_budget_cycle, labor_cycle::*};
use serde_json::json;
fn request(w: &WorldState) -> LaborRequest {
    LaborRequest {
        expected_revision: w.campaign.revision,
        period: "w".into(),
        policies: w.production_data.policies.clone(),
    }
}
#[test]
fn atomic_opening_funds_real_hires_once_and_preserves_people_stocks_and_date() {
    let mut w = common::world(1540);
    let date = w.campaign.clone();
    let stocks = w.stockpiles.clone();
    let total: i64 = w.accounts.iter().map(|a| a.balance.0).sum();
    let r = request(&w);
    let receipt = execute_local_labor(&mut w, &r).unwrap();
    assert_eq!(receipt.hired_fte, 300.);
    assert_eq!(w.pops[0].employed, 300.);
    assert_eq!(w.campaign, date);
    assert_eq!(w.stockpiles, stocks);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_local_labor(&mut w, &r).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn stale_consulted_inputs_reject_before_any_publication() {
    for case in 0..6 {
        let mut w = common::world(1540);
        let prepared = prepare_local_labor(&w, &request(&w)).unwrap();
        match case {
            0 => w.accounts.last_mut().unwrap().balance = Money(1500),
            1 => w.pops[0].size = 999,
            2 => {
                w.production_data.catalog.as_mut().unwrap().methods[0].weekly_productivity =
                    Some(2.)
            }
            3 => w
                .production_data
                .policies
                .taxes
                .insert("s".into(), 0.2)
                .map(|_| ())
                .unwrap_or(()),
            4 => w.production_data.storage_facilities[0].capacity = Some(999.),
            _ => w
                .market_data
                .price_calibration
                .as_mut()
                .unwrap()
                .prices_by_currency
                .get_mut("COIN")
                .unwrap()
                .insert("grain".into(), json!(9))
                .map(|_| ())
                .unwrap_or(()),
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(apply_local_labor(&mut w, prepared).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn partial_receipt_history_and_late_labor_are_rejected() {
    let mut w = common::world(1540);
    let r = request(&w);
    execute_local_labor(&mut w, &r).unwrap();
    w.production_data.committed_operations.clear();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_local_labor(&mut w, &r).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = common::world(1540);
    w.production_data
        .committed_operations
        .push(kingmaker_core::operations::CommittedOperation {
            event_id: "hiring-funding:[\"w\",\"rgo\",\"a\"]".into(),
            value: json!({}),
        });
    assert!(prepare_local_labor(&w, &request(&w)).is_err());
}
#[test]
fn ended_service_is_neither_funded_nor_paid() {
    let mut w = budget::world(4000);
    w.budget_data.employment[0]
        .extra
        .insert("status".into(), json!("ended"));
    let r = budget::request(&w);
    let receipt = execute_budget_cycle(&mut w, &r).unwrap();
    assert!(!receipt.operations.iter().any(|o| o
        .credits
        .iter()
        .any(|leg| leg.owner_id == ExternalId::from("teacher"))));
    assert!(!receipt
        .requirements
        .iter()
        .any(|r| r["sector"] == "education"));
}
#[test]
fn accepted_workers_are_paid_and_produce_in_the_same_period() {
    use kingmaker_core::{market::*, production::*};
    let mut w = common::world(100000);
    w.production_data.storage_facilities.push(serde_json::from_value(json!({"id":"wh","owner_id":"o","province_id":1,"capacity":1000,"unit":"space","status":"source","source":"fixture","goods":{"grain":{"unit":"t","space_per_unit":1}},"access_contracts":[]})).unwrap());
    let r = request(&w);
    let hired = execute_local_labor(&mut w, &r).unwrap();
    let b = kingmaker_core::budget::BudgetRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    };
    execute_budget_cycle(&mut w, &b).unwrap();
    let maintenance = kingmaker_sim::market_cycle::execute_market_phase(
        &mut w,
        &MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: MarketStage::Maintenance,
            profile: Default::default(),
            priorities: None,
        },
    )
    .unwrap();
    let result = kingmaker_sim::production::execute_production_cycle(
        &mut w,
        &ProductionRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
            profile: Default::default(),
            maintenance_receipts: maintenance.maintenance_receipts,
        },
    )
    .unwrap();
    assert_eq!(hired.hired_fte, 600.);
    assert_eq!(result.outputs.iter().map(|o| o.quantity).sum::<f64>(), 600.);
    assert!(w
        .production_data
        .committed_operations
        .iter()
        .any(|o| o.value["type"] == "payroll"));
}
