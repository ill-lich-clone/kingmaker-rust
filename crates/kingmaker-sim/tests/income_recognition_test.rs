#[path = "common/income.rs"]
mod common;
use common::{fixture, fixture_with_rent, oracle, request};
use kingmaker_core::*;
use kingmaker_sim::income_recognition::recognize_producer_income;
use serde_json::json;
#[test]
fn actual_paid_sales_separate_initial_stock_and_real_consumed_input_costs() {
    let (w, basis, opening) = fixture();
    let before = serde_json::to_value(&w).unwrap();
    let rows = recognize_producer_income(&w, &request(), &basis).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].revenue_minor, Money(300));
    assert_eq!(rows[0].opening_stock_revenue_minor, Money(600));
    assert_eq!(rows[0].recognized_cost_minor, Some(Money(160)));
    let expected = oracle(&w, opening);
    assert_eq!(serde_json::to_value(&rows).unwrap(), expected);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn actual_storage_rent_is_recognized_once_from_its_required_production_receipt() {
    let (w, basis, opening) = fixture_with_rent(true);
    let rows = recognize_producer_income(&w, &request(), &basis).unwrap();
    assert_eq!(rows[0].recognized_cost_minor, Some(Money(165)));
    assert_eq!(serde_json::to_value(&rows).unwrap(), oracle(&w, opening));
}
#[test]
fn missing_disguised_or_tampered_required_phase_history_cannot_justify_income() {
    for case in 0..5 {
        let (mut w, basis, _) = fixture();
        match case {
            0 => w.production_data.committed_operations.clear(),
            1 => {
                w.production_data
                    .committed_operations
                    .iter_mut()
                    .find(|op| op.value["type"] == json!("purchase"))
                    .unwrap()
                    .value["paid_minor"] = json!("9999")
            }
            2 => {
                w.production_data
                    .committed_operations
                    .iter_mut()
                    .find(|op| op.value["type"] == json!("production"))
                    .unwrap()
                    .value["type"] = json!("blocked")
            }
            3 => {
                let old = w.market_data.phase_receipts[0].clone();
                w.market_data.phase_receipts.push(old);
            }
            _ => w.production_data.committed_operations.push(kingmaker_core::operations::CommittedOperation{event_id:"orphan-purchase".into(),value:json!({"event_id":"orphan-purchase","type":"purchase","period":"w","paid_minor":"999999"})}),
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(recognize_producer_income(&w, &request(), &basis).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn absent_input_quote_is_unmeasured_instead_of_zero_cost_or_profit() {
    let (mut w, basis, _) = fixture();
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .remove("iron");
    let rows = recognize_producer_income(&w, &request(), &basis).unwrap();
    assert_eq!(rows[0].recognized_cost_minor, None);
    assert!(!rows[0].diagnostics.is_empty());
}
#[test]
fn legitimate_old_history_is_preserved_but_never_counted_as_current_income() {
    let (mut w, basis, _) = fixture();
    w.production_data.committed_operations.push(kingmaker_core::operations::CommittedOperation{event_id:"old".into(),value:json!({"type":"purchase","period":"older","event_id":"old","paid_minor":"99999"})});
    let before = serde_json::to_value(&w).unwrap();
    let rows = recognize_producer_income(&w, &request(), &basis).unwrap();
    assert_eq!(rows[0].revenue_minor, Money(300));
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
