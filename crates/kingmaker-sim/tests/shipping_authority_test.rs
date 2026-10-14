#[path = "common/production.rs"]
mod common;
use kingmaker_core::operations::EconomicOperation;
use kingmaker_sim::operation_session::OperationSession;
use serde_json::json;
#[test]
fn public_shipping_payload_cannot_authorize_goods_money_or_a_phase_boundary() {
    for kind in [
        "shipment_dispatch",
        "shipment_arrival",
        "shipment_interrupted",
        "shipment_recovery",
        "shipment_return_dispatch",
        "shipment_return_arrival",
        "supply_phase_boundary",
        "state_procurement_period",
        "market_phase_boundary",
    ] {
        let world = common::world();
        let op:EconomicOperation=serde_json::from_value(json!({"event_id":"forged","type":kind,"shipment_id":"cargo","debits":[{"owner_id":"o","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"5"}],"credits":[{"owner_id":"new","currency_id":"COIN","purpose":"trade_escrow","amount_minor":"5"}]})).unwrap();
        let mut session = OperationSession::new(&world).unwrap();
        assert!(session.stage(&op).is_err());
        assert!(session.finish().is_err());
    }
}
