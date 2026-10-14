#[path = "common/production.rs"]
mod common;
use kingmaker_core::operations::EconomicOperation;
use kingmaker_sim::operation_session::OperationSession;
use serde_json::json;
#[test]
fn public_construction_payload_never_grants_money_materials_or_progress_authority() {
    for kind in [
        "capitalist_investment",
        "construction_staff_transfer",
        "construction_wait",
        "construction_progress",
        "construction_materials",
        "construction_contract_fee",
        "factory_working_capital",
        "project_refund",
        "construction_cancelled",
        "construction_phase_boundary",
    ] {
        let w = common::world();
        let op:EconomicOperation=serde_json::from_value(json!({"event_id":"forged-construction","type":kind,"project_id":"project","debits":[{"owner_id":"o","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"5"}],"credits":[{"owner_id":"new","currency_id":"COIN","purpose":"construction_escrow","amount_minor":"5"}]})).unwrap();
        let mut s = OperationSession::new(&w).unwrap();
        assert!(s.stage(&op).is_err());
        assert!(s.finish().is_err());
    }
}
