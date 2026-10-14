mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::{market::*, operations::EconomicOperation, production::ProductionRequest};
use kingmaker_sim::{market_cycle::*, production::execute_production_cycle};
use serde_json::json;
#[test]
fn detached_toy_helpers_cannot_execute_economy() {
    let w = market_world();
    let mut rows = vec![kingmaker_sim::market::MarketOrder {
        order_id: 1,
        buyer_province: kingmaker_core::ProvinceId(1),
        goods: kingmaker_core::Goods::Grain,
        requested_qty: 1.0,
        priority: kingmaker_sim::market::OrderPriority::Basic,
        allocated_qty: 0.0,
    }];
    let before = rows.clone();
    assert!(kingmaker_sim::market::clear_market_orders(&mut rows, 100.0).is_err());
    assert_eq!(rows, before);
    assert!(kingmaker_sim::needs::calculate_pop_demands(&w.pops).is_err());
    assert!(
        kingmaker_sim::needs::calculate_pop_demands_with_reserves(&w.pops, &w.stockpiles).is_err()
    );
}
#[test]
fn native_market_sequence_matches_actual_js_operations_balances_and_quantities() {
    for instant in [false, true] {
        let mut w = market_world();
        w.market_data.parameters.as_mut().unwrap().instant_food = Some(instant);
        let source = full_snapshot(&w);
        let req = MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: MarketStage::Maintenance,
            profile: Default::default(),
            priorities: None,
        };
        let mut operations = execute_market_phase(&mut w, &req).unwrap().operations;
        if instant {
            operations.extend(
                execute_market_phase(
                    &mut w,
                    &MarketRequest {
                        stage: MarketStage::EarlyFood,
                        ..req.clone()
                    },
                )
                .unwrap()
                .operations,
            );
        }
        execute_production_cycle(
            &mut w,
            &ProductionRequest {
                expected_revision: 20,
                period: "w".into(),
                profile: Default::default(),
                policies: Default::default(),
                maintenance_receipts: vec![],
            },
        )
        .unwrap();
        operations.extend(
            execute_market_phase(
                &mut w,
                &MarketRequest {
                    stage: MarketStage::ClosingMarket,
                    ..req
                },
            )
            .unwrap()
            .operations,
        );
        let js = oracle(json!({"action":"cycle","snapshot":source,"instant":instant,"period":"w"}));
        let expected: Vec<EconomicOperation> =
            serde_json::from_value(js["operations"].clone()).unwrap();
        let boundaries = operations
            .iter()
            .filter(|o| o.kind == kingmaker_core::operations::OperationKind::MarketPhaseBoundary)
            .collect::<Vec<_>>();
        assert_eq!(boundaries.len(), if instant { 3 } else { 2 });
        assert!(boundaries
            .iter()
            .all(|o| o.debits.is_empty() && o.credits.is_empty() && o.stock_changes.is_empty()));
        operations
            .retain(|o| o.kind != kingmaker_core::operations::OperationKind::MarketPhaseBoundary);
        assert_source_values(
            &serde_json::to_value(operations).unwrap(),
            &serde_json::to_value(expected).unwrap(),
        );
        for a in &w.accounts {
            let js = js["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| {
                    v["owner_id"] == json!(a.owner_id)
                        && v["currency_id"] == json!(a.currency_id)
                        && v["purpose"] == json!(a.purpose)
                })
                .unwrap();
            assert_eq!(
                a.balance.0.to_string(),
                js["amount_minor"].as_str().unwrap()
            );
        }
        for s in &w.stockpiles {
            let js = js["stocks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["opening_stock_id"] == json!(s.external_id))
                .unwrap();
            assert_close(s.quantity, js["amount"].as_f64().unwrap(), 1e-9);
        }
    }
}
