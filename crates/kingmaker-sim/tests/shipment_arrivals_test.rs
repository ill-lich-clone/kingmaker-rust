#[path = "common/shipping.rs"]
mod common;
#[path = "common/income.rs"]
mod income_common;
use kingmaker_core::*;
use kingmaker_sim::shipment_arrivals::{
    apply_shipment_arrivals, execute_shipment_arrivals, prepare_shipment_arrivals,
};
use serde_json::json;
#[test]
fn due_actual_cargo_arrives_pays_seller_and_preserves_all_five_invariants() {
    let mut w = common::world(3.);
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let receipt = execute_shipment_arrivals(&mut w, &common::request("w")).unwrap();
    assert!(receipt.invariants.all_required_passed());
    assert_eq!(common::cash(&w, "seller", "operating_capital"), 1);
    assert_eq!(common::cash(&w, "shipment:cargo", "trade_escrow"), 0);
    assert_eq!(w.stockpiles.len(), 1);
    assert_eq!(w.stockpiles[0].quantity, 3.);
    assert_eq!(w.stockpiles[0].owner_id, ExternalId::from("buyer"));
    assert_eq!(w.stockpiles[0].province_id, ProvinceId(2));
    assert_eq!(w.mobility_data.shipments[0]["status"], json!("arrived"));
    assert_eq!(
        w.mobility_data.shipments[0]["remaining_quantity"].as_f64(),
        Some(0.)
    );
    assert_eq!(
        w.mobility_data.shipments[0]["carrier_release_date"],
        json!({"year":306,"month":1,"day":10})
    );
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    assert_eq!(w.production_data.storage_reservations[0].status, "consumed");
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        execute_shipment_arrivals(&mut w, &common::request("w")).unwrap(),
        receipt
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn three_partial_arrivals_use_accumulated_rounding_and_delayed_carrier_release() {
    let mut w = common::world(1.);
    for (index, paid) in [(1, 0), (2, 1), (3, 1)] {
        w.production_data.storage_facilities[0].capacity = Some(index as f64);
        w.campaign.day = 8 + (index - 1) * 7;
        execute_shipment_arrivals(&mut w, &common::request(&format!("w{index}"))).unwrap();
        assert_eq!(common::cash(&w, "seller", "operating_capital"), paid);
        assert_eq!(w.stockpiles[0].quantity, index as f64);
        assert_eq!(
            w.mobility_data.shipments[0]["remaining_quantity"].as_f64(),
            Some(3. - index as f64)
        );
    }
    assert_eq!(
        w.mobility_data.shipments[0]["carrier_release_date"],
        json!({"year":306,"month":1,"day":24})
    );
    assert_eq!(
        w.mobility_data.shipments[0]["arrival_receipts"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}
#[test]
fn not_due_full_or_unknown_warehouse_preserves_cargo_and_escrow() {
    for case in 0..3 {
        let mut w = common::world(if case == 1 { 0. } else { 3. });
        if case == 0 {
            w.campaign.day = 7;
        } else if case == 2 {
            w.production_data.storage_facilities[0].source = None;
        }
        let accounts = w.accounts.clone();
        let receipt = execute_shipment_arrivals(&mut w, &common::request("w")).unwrap();
        assert_eq!(w.accounts, accounts);
        assert!(w.stockpiles.is_empty());
        assert_eq!(w.mobility_data.shipments[0]["remaining_quantity"], json!(3));
        if case != 0 {
            assert!(!receipt.diagnostics.is_empty());
        }
    }
}
#[test]
fn prepared_staleness_late_missing_reservation_and_altered_history_publish_nothing() {
    for case in 0..4 {
        let mut w = common::world(3.);
        let prepared = prepare_shipment_arrivals(&w, &common::request("w")).unwrap();
        match case {
            0 => w.accounts[2].balance = Money(0),
            1 => w.production_data.storage_facilities[0].owner_id = "other".into(),
            2 => w.mobility_data.shipments[0]["remaining_quantity"] = json!(2),
            _ => w.campaign.day += 1,
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(apply_shipment_arrivals(&mut w, prepared).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
    let mut w = common::world(3.);
    let mut bad = w.mobility_data.shipments[0].clone();
    bad["id"] = json!("bad");
    bad["storage_reservation_id"] = json!("missing");
    w.mobility_data.shipments.push(bad);
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_shipment_arrivals(&mut w, &common::request("w")).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w = common::world(3.);
    execute_shipment_arrivals(&mut w, &common::request("w")).unwrap();
    w.production_data.committed_operations.clear();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_shipment_arrivals(&mut w, &common::request("w")).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn native_snapshot_and_later_date_replay_preserve_required_shipping_history() {
    let mut w = common::world(3.);
    let receipt = execute_shipment_arrivals(&mut w, &common::request("w")).unwrap();
    let mut loaded: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    loaded.rebuild_account_index().unwrap();
    assert_eq!(
        execute_shipment_arrivals(&mut loaded, &common::request("w")).unwrap(),
        receipt
    );
    loaded.campaign.revision += 1;
    loaded.campaign.day += 7;
    assert_eq!(
        execute_shipment_arrivals(&mut loaded, &common::request("w")).unwrap(),
        receipt
    );
}
fn oracle(w: &WorldState, period: &str) -> serde_json::Value {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut snapshot = income_common::snapshot(w);
    snapshot["campaign"]["campaign_id"] = json!(w.campaign.campaign_id);
    snapshot["appliedEventIds"] = json!(w
        .production_data
        .committed_operations
        .iter()
        .map(|o| &o.event_id)
        .collect::<Vec<_>>());
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
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            json!({"action":"arrivals","snapshot":snapshot,"period":period})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn normalize_quantities(value: &mut serde_json::Value) {
    if let Some(rows) = value.as_array_mut() {
        for row in rows {
            normalize_quantities(row);
        }
    } else if let Some(fields) = value.as_object_mut() {
        for (key, v) in fields {
            if matches!(
                key.as_str(),
                "remaining_quantity" | "arrived_quantity" | "returned_quantity" | "quantity"
            ) {
                if let Some(n) = v.as_f64() {
                    *v = json!(n);
                }
            } else {
                normalize_quantities(v);
            }
        }
    }
}
#[test]
fn full_partial_wait_and_empty_arrival_outputs_match_actual_js_source() {
    for capacity in [0., 1., 3.] {
        let mut w = common::world(capacity);
        let mut expected = oracle(&w, "w");
        let receipt = execute_shipment_arrivals(&mut w, &common::request("w")).unwrap();
        let mut source_ops: Vec<kingmaker_core::operations::EconomicOperation> =
            serde_json::from_value(expected["operations"].clone()).unwrap();
        for op in &mut source_ops {
            if let Some(n) = op.meta.get("quantity").and_then(serde_json::Value::as_f64) {
                op.meta.insert("quantity".into(), json!(n));
            }
        }
        assert_eq!(
            &receipt.operations[..receipt.operations.len() - 1],
            source_ops
        );
        normalize_quantities(&mut expected["registry"]["shipments"]);
        let mut actual_shipments = json!(w.mobility_data.shipments);
        normalize_quantities(&mut actual_shipments);
        assert_eq!(actual_shipments, expected["registry"]["shipments"]);
        let reservations: Vec<kingmaker_core::storage::StorageReservation> =
            serde_json::from_value(expected["registry"]["storageReservations"].clone()).unwrap();
        assert_eq!(w.production_data.storage_reservations, reservations);
        assert_eq!(json!(receipt.diagnostics), expected["result"]["blocked"]);
    }
}
#[test]
fn real_return_cargo_restores_original_stock_and_refunds_only_unpaid_escrow() {
    let mut w = common::world(3.);
    w.production_data.storage_facilities[0].id = "source-wh".into();
    w.production_data.storage_facilities[0].owner_id = "seller".into();
    w.production_data.storage_facilities[0].province_id = ProvinceId(1);
    w.production_data.storage_reservations[0].id = "return-reserve".into();
    w.production_data.storage_reservations[0].owner_id = "seller".into();
    w.production_data.storage_reservations[0].facility_id = "source-wh".into();
    w.stockpiles.push(StockpileRecord {
        id: 10,
        external_id: "source-iron".into(),
        owner_id: "seller".into(),
        province_id: ProvinceId(1),
        goods: Goods::Iron,
        quantity: 0.,
        facility_id: Some("source-wh".into()),
        unit: Some(json!("t")),
        ..Default::default()
    });
    let cargo = &mut w.mobility_data.shipments[0];
    cargo["status"] = json!("returning");
    cargo["return_arrival_date"] = json!({"year":306,"month":1,"day":8});
    cargo["return_initial_quantity"] = json!(3);
    cargo["return_facility_id"] = json!("source-wh");
    cargo["return_storage_reservation_id"] = json!("return-reserve");
    cargo["returned_quantity"] = json!(0);
    cargo["refunded_minor"] = json!("0");
    w.production_data.storage_facilities[0].owner_id="landlord".into();
    w.production_data.storage_reservations[0].lease_terms=Some(serde_json::from_value(json!({"owner_id":"seller","currency_id":"COIN","purpose":"operating_capital","price_minor_per_space":"0","status":"source","source":"fixture"})).unwrap());
    let expected = oracle(&w, "w");
    let receipt = execute_shipment_arrivals(&mut w, &common::request("w")).unwrap();
    assert_eq!(w.stockpiles[0].quantity, 3.);
    assert_eq!(common::cash(&w, "buyer", "personal_savings"), 1);
    assert_eq!(common::cash(&w, "seller", "operating_capital"), 0);
    assert_eq!(w.mobility_data.shipments[0]["status"], json!("returned"));
    assert_eq!(
        w.mobility_data.shipments[0]["carrier_release_date"],
        json!({"year":306,"month":1,"day":8})
    );
    let mut ops: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(expected["operations"].clone()).unwrap();
    for op in &mut ops {
        op.meta.insert(
            "quantity".into(),
            json!(op.meta["quantity"].as_f64().unwrap()),
        );
    }
    assert_eq!(&receipt.operations[..receipt.operations.len() - 1], ops);
}

#[test]
fn remediation_internal_phase_no_global_fingerprint() {
    use std::{cell::RefCell,rc::Rc,io::{self,Write}};
    #[derive(Clone)]struct Capture(Rc<RefCell<Vec<u8>>>);
    impl Write for Capture{fn write(&mut self,b:&[u8])->io::Result<usize>{self.0.borrow_mut().extend_from_slice(b);Ok(b.len())}fn flush(&mut self)->io::Result<()>{Ok(())}}
    let data=Rc::new(RefCell::new(Vec::new()));
    let session=kingmaker_core::diagnostics::ProfileSession::install(Box::new(Capture(data.clone()))).unwrap();
    let mut w=common::world(1.);
    execute_shipment_arrivals(&mut w,&common::request("local")).unwrap();
    session.finish().unwrap();
    let events=String::from_utf8(data.borrow().clone()).unwrap();
    let globals=events.lines().filter(|l|serde_json::from_str::<serde_json::Value>(l).unwrap()["name"]=="guard.world").count();
    assert_eq!(globals,0,"local prepare/apply serialized whole world");
    assert_eq!(w.stockpiles[0].quantity,1.);
    assert_eq!(common::cash(&w,"seller","operating_capital"),0);
}
