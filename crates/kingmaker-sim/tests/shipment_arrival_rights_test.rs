#[path = "common/shipping.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::shipment_arrivals::execute_shipment_arrivals;
use serde_json::json;
#[test]
fn leased_destination_retains_paid_reservation_terms_and_missing_tenancy_rejects() {
    for allowed in [true, false] {
        let mut w = common::world(3.);
        w.production_data.storage_facilities[0].owner_id = "landlord".into();
        if allowed {
            w.production_data.storage_reservations[0].lease_terms=Some(serde_json::from_value(json!({"owner_id":"buyer","currency_id":"COIN","purpose":"personal_savings","price_minor_per_space":"0","status":"source","source":"fixture","cancellation_policy":"refund_unused"})).unwrap());
        }
        let before = serde_json::to_value(&w).unwrap();
        let result = execute_shipment_arrivals(&mut w, &common::request("w"));
        if allowed {
            assert!(result.is_ok());
            assert_eq!(w.stockpiles[0].owner_id, ExternalId::from("buyer"));
            assert_eq!(
                w.production_data.storage_facilities[0].owner_id,
                ExternalId::from("landlord")
            );
            assert!(w.production_data.storage_reservations[0]
                .lease_terms
                .is_some());
        } else {
            assert!(result.is_err());
            assert_eq!(serde_json::to_value(&w).unwrap(), before);
        }
    }
}
#[test]
fn empty_phase_is_canonical_and_receipt_registry_or_date_tampering_rejects() {
    for case in 0..3 {
        let mut w = common::world(3.);
        w.mobility_data.shipments.clear();
        w.production_data.storage_reservations.clear();
        let receipt = execute_shipment_arrivals(&mut w, &common::request("w")).unwrap();
        assert_eq!(receipt.operations.len(), 1);
        match case {
            0 => w.transport_data.phase_receipts.clear(),
            1 => {
                w.transport_data.phase_receipts[0].registry_changes["shipments"] =
                    json!([{"bogus":true}])
            }
            _ => w.transport_data.phase_receipts[0].date.day = 9,
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_shipment_arrivals(&mut w, &common::request("w")).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn partial_arrivals_move_producer_contributions_from_cargo_to_actual_stock_once() {
    let mut w = common::world(1.);
    let mut method = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    method.method_id = "artisan:steel".into();
    method.kind = "artisan".into();
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .push(method);
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"artisan","owner_id":"seller","province_id":1,"method_id":"artisan:steel","assignments":[]})).unwrap());
    let mapping = w.production_data.storage_facilities[0]
        .goods
        .remove("iron")
        .unwrap();
    w.production_data.storage_facilities[0]
        .goods
        .insert("steel".into(), mapping);
    w.production_data.storage_reservations[0].goods_id = "steel".into();
    w.mobility_data.shipments[0]["goods_id"] = json!("steel");
    w.mobility_data.shipments[0]["goods_origin"] = json!("produced");
    w.mobility_data.shipments[0]["producer_contributions"] =
        json!([{"producer_id":"artisan","quantity":1.5}]);
    for index in 1..=3 {
        w.production_data.storage_facilities[0].capacity = Some(index as f64);
        w.campaign.day = 8 + (index - 1) * 7;
        execute_shipment_arrivals(&mut w, &common::request(&format!("w{index}"))).unwrap();
        assert_eq!(w.income_data.stock_contributions.len(), 1);
        assert_eq!(
            w.income_data.stock_contributions[0].quantity,
            index as f64 * 0.5
        );
        assert_eq!(
            w.mobility_data.shipments[0]["producer_contributions"][0]["quantity"].as_f64(),
            Some((3 - index) as f64 * 0.5)
        );
    }
}
