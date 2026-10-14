#[path = "common/market.rs"]
mod common;
use kingmaker_core::market::*;
use kingmaker_sim::market_cycle::execute_market_phase;
use serde_json::json;
#[test]
fn stored_orders_and_empty_phase_cannot_be_changed_without_canonical_history() {
    for case in 0..3 {
        let mut w = common::market_world();
        let r = MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: MarketStage::Maintenance,
            profile: Default::default(),
            priorities: None,
        };
        execute_market_phase(&mut w, &r).unwrap();
        match case {0=>w.market_data.phase_receipts[0].orders.push(serde_json::from_value(json!({"id":"forged","owner_id":"a","province_id":1,"currency_id":"COIN","purpose":"personal_savings","goods_id":"iron","quantity":100,"priority":"production_inputs","consume":false})).unwrap()),1=>w.market_data.phase_receipts[0].frozen_needs.push(ResolvedNeed{need_id:"forged".into(),quantity:Some(1.),gross_quantity:Some(1.),extra:Default::default()}),_=>w.market_data.phase_receipts.clear()};
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_market_phase(&mut w, &r).is_err(), "case{case}");
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
