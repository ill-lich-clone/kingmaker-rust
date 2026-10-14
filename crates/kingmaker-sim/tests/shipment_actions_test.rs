#[path = "common/shipping.rs"]
mod common;
#[path = "common/income.rs"]
mod income_common;
use kingmaker_core::{transport::*, *};
use kingmaker_sim::shipment_actions::{
    apply_shipment_action, execute_shipment_action, prepare_shipment_action,
};
use serde_json::json;
fn interrupt() -> ShipmentActionRequest {
    ShipmentActionRequest {
        expected_revision: 20,
        event_id: "interrupt".into(),
        shipment_id: "cargo".into(),
        actor_id: None,
        action: ShipmentAction::Interrupt {
            reason: "closed road".into(),
        },
    }
}
fn action(returning: bool) -> ShipmentActionRequest {
    ShipmentActionRequest {
        expected_revision: 20,
        event_id: if returning { "return" } else { "resume" }.into(),
        shipment_id: "cargo".into(),
        actor_id: Some("buyer".into()),
        action: if returning {
            ShipmentAction::Return {
                recovery_id: "approved".into(),
            }
        } else {
            ShipmentAction::Resume {
                recovery_id: "approved".into(),
            }
        },
    }
}
fn world(returning: bool) -> WorldState {
    let mut w = common::world(3.);
    w.accounts[4].balance = Money(10);
    let mut source = w.production_data.storage_facilities[0].clone();
    source.id = "source".into();
    source.province_id = ProvinceId(1);
    source.owner_id = "seller".into();
    w.production_data.storage_facilities.push(source);
    w.stockpiles.push(StockpileRecord {
        id: 10,
        external_id: "source-iron".into(),
        owner_id: "seller".into(),
        province_id: ProvinceId(1),
        goods: Goods::Iron,
        quantity: 0.,
        unit: Some(json!("t")),
        facility_id: Some("source".into()),
        currency_id: Some("COIN".into()),
        inventory_origin: Some("opening".into()),
        ..Default::default()
    });
    w.transport_data.shipment_recovery_plans = vec![
        json!({"id":"approved","shipment_id":"cargo","action":if returning{"return"}else{"resume"},"status":"approved","source":"author","approval_ref":"author","accepted_buyer_id":"buyer","accepted_carrier_id":"carrier","accepted_seller_id":"seller","currency_id":"COIN","remaining_travel_days":4,"extra_freight_minor":"2","source_facility_id":"source","unknown_author_field":"keep"}),
    ];
    w
}
#[test]
fn interruption_preserves_actual_cargo_escrow_rights_and_has_safe_identity() {
    let mut w = world(false);
    let goods = w.stockpiles.clone();
    let accounts = w.accounts.clone();
    let receipt = execute_shipment_action(&mut w, &interrupt()).unwrap();
    assert!(receipt.invariants.all_required_passed());
    assert_eq!(w.mobility_data.shipments[0]["status"], json!("interrupted"));
    assert_eq!(w.mobility_data.shipments[0]["remaining_quantity"], json!(3));
    assert_eq!(w.stockpiles, goods);
    assert_eq!(w.accounts, accounts);
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        execute_shipment_action(&mut w, &interrupt()).unwrap(),
        receipt
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut wrong = interrupt();
    wrong.action = ShipmentAction::Interrupt {
        reason: "different".into(),
    };
    assert!(execute_shipment_action(&mut w, &wrong).is_err());
}
#[test]
fn approved_resume_pays_actual_extra_freight_and_advances_only_cargo_dates() {
    let mut w = world(false);
    execute_shipment_action(&mut w, &interrupt()).unwrap();
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let receipt = execute_shipment_action(&mut w, &action(false)).unwrap();
    assert!(receipt.invariants.all_required_passed());
    assert_eq!(common::cash(&w, "buyer", "personal_savings"), 8);
    assert_eq!(common::cash(&w, "carrier", "operating_capital"), 2);
    assert_eq!(common::cash(&w, "shipment:cargo", "trade_escrow"), 1);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    assert_eq!(
        w.mobility_data.shipments[0]["arrival_date"],
        json!({"year":306,"month":1,"day":12})
    );
    assert_eq!(
        w.mobility_data.shipments[0]["carrier_release_date"],
        json!({"year":306,"month":1,"day":14})
    );
    assert_eq!(w.campaign.day, 8);
    let mut loaded: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    loaded.rebuild_account_index().unwrap();
    loaded.campaign.revision += 1;
    loaded.campaign.day += 1;
    assert_eq!(
        execute_shipment_action(&mut loaded, &action(false)).unwrap(),
        receipt
    );
}
#[test]
fn physical_return_reserves_source_releases_destination_then_refunds_after_real_arrival() {
    let mut w = world(true);
    execute_shipment_action(&mut w, &interrupt()).unwrap();
    execute_shipment_action(&mut w, &action(true)).unwrap();
    assert_eq!(w.mobility_data.shipments[0]["status"], json!("returning"));
    assert_eq!(w.stockpiles[0].quantity, 0.);
    assert_eq!(common::cash(&w, "buyer", "personal_savings"), 8);
    assert_eq!(common::cash(&w, "shipment:cargo", "trade_escrow"), 1);
    assert_eq!(w.production_data.storage_reservations[0].status, "released");
    assert_eq!(w.production_data.storage_reservations[1].status, "held");
    assert_eq!(
        w.production_data.storage_reservations[1].owner_id,
        ExternalId::from("seller")
    );
    assert_eq!(w.production_data.storage_reservations[1].quantity, 3.);
    w.campaign.day = 12;
    w.campaign.revision += 1;
    kingmaker_sim::shipment_arrivals::execute_shipment_arrivals(
        &mut w,
        &TransportRequest {
            expected_revision: 21,
            period: "arrival".into(),
            stage: TransportStage::Arrivals,
        },
    )
    .unwrap();
    assert_eq!(w.stockpiles[0].quantity, 3.);
    assert_eq!(common::cash(&w, "buyer", "personal_savings"), 9);
    assert_eq!(common::cash(&w, "shipment:cargo", "trade_escrow"), 0);
    assert_eq!(w.mobility_data.shipments[0]["status"], json!("returned"));
}
#[test]
fn missing_approvals_wrong_actor_currency_consent_or_unfunded_freight_reject_without_changes() {
    for case in 0..5 {
        let mut w = world(true);
        execute_shipment_action(&mut w, &interrupt()).unwrap();
        let mut r = action(true);
        match case {
            0 => r.actor_id = Some("thief".into()),
            1 => w.transport_data.shipment_recovery_plans[0]["approval_ref"] = json!(null),
            2 => w.transport_data.shipment_recovery_plans[0]["currency_id"] = json!("OTHER"),
            3 => w.transport_data.shipment_recovery_plans[0]["accepted_seller_id"] = json!("other"),
            _ => w.accounts[4].balance = Money(0),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_shipment_action(&mut w, &r).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn stale_readset_late_destination_release_failure_and_lost_history_are_atomic() {
    let mut w = world(true);
    execute_shipment_action(&mut w, &interrupt()).unwrap();
    let prepared = prepare_shipment_action(&w, &action(true)).unwrap();
    w.transport_data.shipment_recovery_plans[0]["extra_freight_minor"] = json!("3");
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_shipment_action(&mut w, prepared).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = world(true);
    execute_shipment_action(&mut w, &interrupt()).unwrap();
    w.production_data.storage_reservations[0].status = "released".into();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_shipment_action(&mut w, &action(true)).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = world(false);
    execute_shipment_action(&mut w, &interrupt()).unwrap();
    execute_shipment_action(&mut w, &action(false)).unwrap();
    w.production_data
        .committed_operations
        .retain(|o| o.event_id != ExternalId::from("resume"));
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_shipment_action(&mut w, &action(false)).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn source_interruption_resume_and_return_operations_and_cargo_fields_match_js() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for command in ["interrupt", "resume", "return"] {
        let mut w = world(command == "return");
        if command != "interrupt" {
            execute_shipment_action(&mut w, &interrupt()).unwrap();
        }
        let req = if command == "interrupt" {
            interrupt()
        } else {
            action(command == "return")
        };
        let mut snapshot = income_common::snapshot(&w);
        snapshot["campaign"]["campaign_id"] = json!(w.campaign.campaign_id);
        snapshot["registry"]["shipmentRecoveryPlans"] =
            json!(w.transport_data.shipment_recovery_plans);
        let mut child = Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/tests/shipping-oracle.cjs"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(json!({"action":command,"snapshot":snapshot,"actor":req.actor_id,"request":{"eventId":req.event_id,"shipmentId":"cargo","reason":"closed road","recoveryId":"approved"}}).to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let expected: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        let receipt = execute_shipment_action(&mut w, &req).unwrap();
        let ops: Vec<kingmaker_core::operations::EconomicOperation> =
            serde_json::from_value(expected["operations"].clone()).unwrap();
        assert_eq!(&receipt.operations[..receipt.operations.len() - 1], ops);
        let mut actual = w.mobility_data.shipments[0].clone();
        let mut expected = expected["registry"]["shipments"][0].clone();
        if command == "return" {
            actual["return_initial_quantity"] =
                json!(actual["return_initial_quantity"].as_f64().unwrap());
            expected["return_initial_quantity"] =
                json!(expected["return_initial_quantity"].as_f64().unwrap());
        }
        assert_eq!(actual, expected);
    }
}
#[test]
fn return_after_partial_delivery_keeps_delivered_goods_and_refunds_only_remaining_value() {
    let mut w = world(true);
    w.mobility_data.shipments[0]["goods_price_minor"] = json!("3");
    w.accounts[2].balance = Money(3);
    w.production_data.storage_facilities[0].capacity = Some(1.);
    kingmaker_sim::shipment_arrivals::execute_shipment_arrivals(
        &mut w,
        &common::request("partial"),
    )
    .unwrap();
    assert_eq!(common::cash(&w, "seller", "operating_capital"), 1);
    assert_eq!(common::cash(&w, "shipment:cargo", "trade_escrow"), 2);
    execute_shipment_action(&mut w, &interrupt()).unwrap();
    execute_shipment_action(&mut w, &action(true)).unwrap();
    w.campaign.day = 12;
    w.campaign.revision += 1;
    kingmaker_sim::shipment_arrivals::execute_shipment_arrivals(
        &mut w,
        &TransportRequest {
            expected_revision: 21,
            period: "return-arrival".into(),
            stage: TransportStage::Arrivals,
        },
    )
    .unwrap();
    assert_eq!(common::cash(&w, "buyer", "personal_savings"), 10);
    assert_eq!(common::cash(&w, "seller", "operating_capital"), 1);
    assert_eq!(
        w.stockpiles
            .iter()
            .filter(|s| s.owner_id == ExternalId::from("seller"))
            .map(|s| s.quantity)
            .sum::<f64>(),
        2.
    );
    assert_eq!(
        w.stockpiles
            .iter()
            .filter(|s| s.owner_id == ExternalId::from("buyer"))
            .map(|s| s.quantity)
            .sum::<f64>(),
        1.
    );
}
#[test]
fn return_refunds_unused_destination_rent_and_records_unpaid_provider_debt() {
    let mut w = world(true);
    w.production_data.storage_facilities[0].owner_id = "landlord".into();
    let lease=serde_json::from_value(json!({"owner_id":"buyer","currency_id":"COIN","purpose":"personal_savings","price_minor_per_space":"1","status":"source","source":"fixture","cancellation_policy":"refund_unused"})).unwrap();
    w.production_data.storage_reservations[0].lease_terms = Some(lease);
    w.production_data.storage_reservations[0].fee_paid_minor = Some(Money(3));
    w.accounts.push(AccountBalance {
        id: AccountId(20),
        external_id: "landlord-cash".into(),
        owner_id: "landlord".into(),
        currency_id: "COIN".into(),
        purpose: "operating_capital".into(),
        balance: Money(2),
    });
    w.rebuild_account_index().unwrap();
    execute_shipment_action(&mut w, &interrupt()).unwrap();
    let receipt = execute_shipment_action(&mut w, &action(true)).unwrap();
    assert!(receipt.invariants.all_required_passed());
    assert_eq!(common::cash(&w, "buyer", "personal_savings"), 10);
    assert_eq!(common::cash(&w, "landlord", "operating_capital"), 0);
    assert_eq!(w.production_data.obligations.len(), 1);
    assert_eq!(w.production_data.obligations[0].amount_minor, Money(1));
}
