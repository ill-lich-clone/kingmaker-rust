#[path = "common/production.rs"]
mod common;
use kingmaker_core::operations::CommittedOperation;
use kingmaker_core::production::MaintenanceReceipt;
use kingmaker_sim::{operation_session::*, production_validation::*};
use serde_json::json;
#[test]
fn source_consumption_proof_requires_current_pop_quantities_and_unique_orders() {
    let mut w = common::world();
    let event = "market:fixture:w:maintenance:food";
    w.production_data.committed_operations.push(CommittedOperation{event_id:event.into(),value:json!({"event_id":event,"type":"purchase","pop_id":"p","maintenance":true,"consumed":true,"need_id":"food","order_id":"food","quantity":1,"requested_quantity":2,"stock_changes":[{"stock_id":"food","delta":-1}]})});
    let s = OperationSession::new(&w).unwrap();
    let req = common::request();
    let mut receipt: MaintenanceReceipt = serde_json::from_value(
        json!({"pop_id":"p","period":"w","fraction":0.5,"event_ids":[event]}),
    )
    .unwrap();
    assert!(maintenance_verified(&s, &req, &receipt));
    receipt.fraction = 1.0;
    assert!(!maintenance_verified(&s, &req, &receipt));
    receipt.fraction = 0.5;
    receipt.period = Some("old".into());
    assert!(!maintenance_verified(&s, &req, &receipt));
    receipt.period = Some("w".into());
    receipt.pop_id = "other".into();
    assert!(!maintenance_verified(&s, &req, &receipt));
    receipt.pop_id = "p".into();
    receipt.event_ids.push(event.into());
    assert!(!maintenance_verified(&s, &req, &receipt));
}
