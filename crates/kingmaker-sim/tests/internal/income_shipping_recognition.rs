//! Formula unit tests only: this does not execute or prove the deferred shipping phase.
#[path = "../common/production.rs"]
mod common;
use crate::{
    income_basis::capture_income_opening, income_recognition::recognize_producer_operations,
    operation_session::OperationSession,
};
use kingmaker_core::{income::IncomeRequest, operations::*, *};
use serde_json::json;

fn request() -> IncomeRequest {
    IncomeRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    }
}
fn operation(
    kind: &str,
    id: &str,
    owner: &str,
    currency: &str,
    amount: &str,
    credit: bool,
) -> EconomicOperation {
    let mut v = json!({"event_id":id,"type":kind,"shipment_id":"cargo","goods_origin":"produced","period":"w"});
    v[if credit { "credits" } else { "debits" }] = json!([{"owner_id":owner,"currency_id":currency,"purpose":"employer_operating_capital","amount_minor":amount}]);
    serde_json::from_value(v).unwrap()
}
#[test]
fn arrival_origin_freight_recovery_and_return_cost_are_counted_once_per_currency() {
    let mut w = common::world();
    let basis = capture_income_opening(&w, &request()).unwrap();
    w.mobility_data
        .shipments
        .push(json!({"id":"cargo","buyer_id":"o","currency_id":"COIN","freight_paid_minor":"10"}));
    let mut initial = operation("shipment_arrival", "initial", "o", "COIN", "30", true);
    initial.meta.insert("goods_origin".into(), json!("opening"));
    let operations = vec![
        operation("shipment_arrival", "arrived", "o", "COIN", "100", true),
        initial,
        operation("shipment_arrival", "foreign", "o", "OTHER", "50", true),
        operation("shipment_dispatch", "dispatch", "o", "COIN", "10", false),
        operation("shipment_recovery", "recover", "o", "COIN", "4", false),
        operation(
            "shipment_return_dispatch",
            "return",
            "o",
            "COIN",
            "3",
            false,
        ),
    ];
    let before = serde_json::to_value(&w).unwrap();
    let rows = recognize_producer_operations(&w, &basis, &operations).unwrap();
    assert_eq!(rows[0].revenue_minor, Money(100));
    assert_eq!(rows[0].opening_stock_revenue_minor, Money(30));
    assert_eq!(rows[0].recognized_cost_minor, Some(Money(17)));
    assert_eq!(rows[1].currency_id, ExternalId::from("OTHER"));
    assert_eq!(rows[1].revenue_minor, Money(50));
    assert_eq!(rows[1].recognized_cost_minor, Some(Money(0)));
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn unimplemented_transport_payload_has_no_public_financial_authority() {
    for kind in [
        "shipment_arrival",
        "shipment_dispatch",
        "shipment_recovery",
        "shipment_return_dispatch",
    ] {
        let w = common::world();
        let mut op = operation(kind, "forged-shipment", "o", "COIN", "1", false);
        op.credits = vec![MoneyLeg {
            owner_id: "o".into(),
            currency_id: "COIN".into(),
            purpose: "operating_capital".into(),
            amount_minor: Money(1),
        }];
        let mut s = OperationSession::new(&w).unwrap();
        assert!(s.stage(&op).is_err());
        assert!(s.finish().is_err());
    }
}
#[test]
fn cumulative_income_overflow_rejects_before_any_mutation() {
    let w = common::world();
    let basis = capture_income_opening(&w, &request()).unwrap();
    let ops = vec![
        operation(
            "shipment_arrival",
            "a",
            "o",
            "COIN",
            &i64::MAX.to_string(),
            true,
        ),
        operation("shipment_arrival", "b", "o", "COIN", "1", true),
    ];
    assert!(recognize_producer_operations(&w, &basis, &ops).is_err());
}
