#[path = "common/physical_supply.rs"]
mod common;
#[path = "common/income.rs"]
mod income_common;
use kingmaker_core::{transport::*, *};
use kingmaker_sim::local_food_reserves::{
    apply_local_food_reserves, execute_local_food_reserves, prepare_local_food_reserves,
};
use serde_json::json;
fn request() -> TransportRequest {
    TransportRequest {
        stage: TransportStage::LocalFoodReserves,
        ..common::request()
    }
}
fn ready(local: bool) -> WorldState {
    common::ready_through_market(|w| {
        let needs = w.market_data.needs.as_mut().unwrap();
        let norm = &mut needs.profiles[0].norms[0];
        norm.need_id = "food".into();
        norm.extra.insert(
            "alternatives".into(),
            json!([{"goods_id":"grain","share":1}]),
        );
        norm.extra
            .insert("quantity_per_person_week".into(), json!(0.007));
        w.accounts[0].balance = Money(200);
        w.stockpiles[0].goods = Goods::Grain;
        w.stockpiles[0].quantity = 21.;
        w.market_data.sale_offers[0].goods_id = "grain".into();
        w.market_data
            .price_calibration
            .as_mut()
            .unwrap()
            .prices_by_currency
            .get_mut("COIN")
            .unwrap()
            .insert("grain".into(), json!(0.001));
        for f in &mut w.production_data.storage_facilities {
            let m = f.goods.remove("iron").unwrap();
            f.goods.insert("grain".into(), m);
        }
        let m = w.transport_data.supply_routes[0]["goods"]["iron"].clone();
        w.transport_data.supply_routes[0]["goods"] = json!({"grain":m});
        if local {
            for p in &mut w.market_data.territory {
                p.region_id = Some("local".into());
            }
        }
    })
}
#[test]
fn actual_local_reserve_purchase_is_paid_stored_and_not_consumed_twice() {
    let mut w = ready(true);
    assert_eq!(w.stockpiles[0].quantity, 14.);
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let receipt = execute_local_food_reserves(&mut w, &request()).unwrap();
    assert!(receipt.invariants.production_required_passed());
    assert_eq!(w.stockpiles[0].quantity, 7.);
    let reserve = w
        .stockpiles
        .iter()
        .find(|s| s.owner_id == ExternalId::from("a") && s.purpose == "food_reserve")
        .unwrap();
    assert_eq!(reserve.quantity, 7.);
    assert_eq!(reserve.beneficiary_pop_id, Some("a".into()));
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    assert_eq!(w.accounts[0].balance, Money(198));
    assert!(receipt
        .operations
        .iter()
        .any(|o| o.meta.get("consumed") == Some(&json!(false))));
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        execute_local_food_reserves(&mut w, &request()).unwrap(),
        receipt
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut loaded: WorldState = serde_json::from_value(before).unwrap();
    loaded.rebuild_account_index().unwrap();
    assert_eq!(
        execute_local_food_reserves(&mut loaded, &request()).unwrap(),
        receipt
    );
}
#[test]
fn outgoing_supply_requires_the_actual_local_food_reserve_phase() {
    let mut w = ready(false);
    let before = serde_json::to_value(&w).unwrap();
    assert!(
        kingmaker_sim::physical_supply::execute_physical_supply(&mut w, &common::request())
            .is_err()
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    execute_local_food_reserves(&mut w, &request()).unwrap();
    w.transport_data
        .phase_receipts
        .last_mut()
        .unwrap()
        .decisions
        .push(json!("fake"));
    let before = serde_json::to_value(&w).unwrap();
    assert!(
        kingmaker_sim::physical_supply::execute_physical_supply(&mut w, &common::request())
            .is_err()
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn reserve_receipt_empty_history_and_stale_inputs_cannot_be_bypassed() {
    let mut w = ready(false);
    let p = prepare_local_food_reserves(&w, &request()).unwrap();
    w.accounts[0].balance = Money(0);
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_local_food_reserves(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    for case in 0..3 {
        let mut w = ready(false);
        execute_local_food_reserves(&mut w, &request()).unwrap();
        match case {
            0 => w.production_data.committed_operations.retain(|o| {
                !o.event_id
                    .as_str()
                    .unwrap_or("")
                    .ends_with(":local-food-reserves")
            }),
            1 => w
                .transport_data
                .phase_receipts
                .last_mut()
                .unwrap()
                .decisions
                .push(json!("forged")),
            _ => w
                .transport_data
                .phase_receipts
                .retain(|p| p.request.stage != TransportStage::LocalFoodReserves),
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_local_food_reserves(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn food_foreign_supply_uses_lead_time_and_dated_feedback_instead_of_one_week() {
    let mut w = ready(false);
    w.transport_data.transport_assets[0]["capacity"] = json!(20);
    for k in [
        "capacity_per_week",
        "source_handling_per_week",
        "destination_handling_per_week",
    ] {
        w.transport_data.supply_routes[0][k] = json!(20);
    }
    execute_local_food_reserves(&mut w, &request()).unwrap();
    let receipt =
        kingmaker_sim::physical_supply::execute_physical_supply(&mut w, &common::request())
            .unwrap();
    assert_eq!(w.mobility_data.shipments.len(), 1);
    assert_eq!(
        w.mobility_data.shipments[0]["remaining_quantity"].as_f64(),
        Some(14.)
    );
    assert_eq!(
        w.mobility_data.shipments[0]["inventory_purpose"],
        json!("food_reserve")
    );
    assert_eq!(w.accounts[0].balance, Money(32));
    assert_eq!(w.stockpiles[0].quantity, 7.);
    assert!(!w
        .stockpiles
        .iter()
        .any(|s| s.owner_id == ExternalId::from("a") && s.quantity > 0.));
    assert!(receipt
        .diagnostics
        .iter()
        .any(|d| d["food_forecast"][0]["uncovered_before_arrival"] == json!(7.)));
}
#[test]
fn local_food_reserve_payment_quantity_and_ids_match_actual_js_market() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut w = ready(true);
    let mut snapshot = income_common::snapshot(&w);
    snapshot["campaign"]["campaign_id"] = json!(w.campaign.campaign_id);
    snapshot["registry"]["consumerFacilityByProvince"] =
        json!(w.market_data.consumer_facility_by_province);
    let orders = w
        .market_data
        .phase_receipts
        .iter()
        .find(|p| p.request.stage == kingmaker_core::market::MarketStage::ClosingMarket)
        .unwrap()
        .orders
        .clone();
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
            json!({"action":"food-reserves","snapshot":snapshot,"period":"w","orders":orders})
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
    let expected: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut ops: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(expected["operations"].clone()).unwrap();
    for op in &mut ops {
        for key in ["quantity", "requested_quantity"] {
            if let Some(q) = op.meta.get(key).and_then(serde_json::Value::as_f64) {
                op.meta.insert(key.into(), json!(q));
            }
        }
    }
    let receipt = execute_local_food_reserves(&mut w, &request()).unwrap();
    assert_eq!(&receipt.operations[..receipt.operations.len() - 1], ops);
}
#[test]
fn two_food_routes_use_dispatch_feedback_and_shared_fleet_with_actual_js_parity() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut w = ready(false);
    w.accounts[0].balance = Money(1000);
    w.transport_data.transport_assets[0]["capacity"] = json!(20);
    for k in [
        "capacity_per_week",
        "source_handling_per_week",
        "destination_handling_per_week",
    ] {
        w.transport_data.supply_routes[0][k] = json!(20);
    }
    let mut route = w.transport_data.supply_routes[0].clone();
    route["id"] = json!("long");
    route["travel_days"] = json!(14);
    w.transport_data.supply_routes.push(route);
    execute_local_food_reserves(&mut w, &request()).unwrap();
    let mut snapshot = income_common::snapshot(&w);
    snapshot["campaign"]["campaign_id"] = json!(w.campaign.campaign_id);
    snapshot["registry"]["transportAssets"] = json!(w.transport_data.transport_assets);
    snapshot["registry"]["supplyRoutes"] = json!(w.transport_data.supply_routes);
    snapshot["registry"]["transportTerminals"] = json!(w.transport_data.transport_terminals);
    snapshot["registry"]["consumerFacilityByProvince"] =
        json!(w.market_data.consumer_facility_by_province);
    snapshot["appliedEventIds"] = json!(w
        .production_data
        .committed_operations
        .iter()
        .map(|o| &o.event_id)
        .collect::<Vec<_>>());
    let market = &w.market_data.phase_receipts[0];
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
    child.stdin.take().unwrap().write_all(json!({"action":"physical","active":true,"snapshot":snapshot,"period":"w","orders":market.orders,"foodOrders":market.orders,"marketOperations":market.operations}).to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let js: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let receipt =
        kingmaker_sim::physical_supply::execute_physical_supply(&mut w, &common::request())
            .unwrap();
    let ops: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(js["operations"].clone()).unwrap();
    assert_eq!(
        &receipt.operations[..receipt.operations.len() - 1],
        ops,
        "JS result: {}",
        js
    );
    assert_eq!(w.mobility_data.shipments.len(), 2);
    assert_eq!(
        w.mobility_data.shipments[0]["remaining_quantity"].as_f64(),
        Some(14.)
    );
    assert_eq!(
        w.mobility_data.shipments[1]["remaining_quantity"].as_f64(),
        Some(6.)
    );
    assert_eq!(w.stockpiles[0].quantity, 1.);
    assert_eq!(w.accounts[0].balance, Money(760));
    let expected: Vec<kingmaker_sim::food_supply_forecast::FoodForecastDiagnostic> =
        serde_json::from_value(js["result"]["food_forecast"].clone()).unwrap();
    let actual: Vec<kingmaker_sim::food_supply_forecast::FoodForecastDiagnostic> =
        serde_json::from_value(receipt.diagnostics.last().unwrap()["food_forecast"].clone())
            .unwrap();
    assert_eq!(actual, expected);
}
