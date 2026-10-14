#[path = "common/physical_supply.rs"]
mod common;
#[path = "common/income.rs"]
mod income_common;
use kingmaker_core::*;
use kingmaker_sim::physical_supply::{
    apply_physical_supply, execute_physical_supply, prepare_physical_supply,
};
use serde_json::json;
#[test]
fn autonomous_supply_uses_actual_unfilled_weekly_need_then_real_dated_arrival() {
    let mut w = common::ready();
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let receipt = execute_physical_supply(&mut w, &common::request()).unwrap();
    assert!(receipt.invariants.all_required_passed());
    assert_eq!(w.mobility_data.shipments.len(), 1);
    assert_eq!(
        w.mobility_data.shipments[0]["remaining_quantity"].as_f64(),
        Some(1.)
    );
    assert_eq!(w.stockpiles[0].quantity, 4.);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    assert_eq!(
        w.account_by_address(&AccountAddress {
            owner_id: "a".into(),
            currency_id: "COIN".into(),
            purpose: "personal_savings".into()
        })
        .unwrap()
        .balance,
        Money(18)
    );
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        execute_physical_supply(&mut w, &common::request()).unwrap(),
        receipt
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut loaded: WorldState = serde_json::from_value(before).unwrap();
    loaded.rebuild_account_index().unwrap();
    assert_eq!(
        execute_physical_supply(&mut loaded, &common::request()).unwrap(),
        receipt
    );
    loaded.campaign.day = 8;
    loaded.campaign.revision += 1;
    kingmaker_sim::shipment_arrivals::execute_shipment_arrivals(
        &mut loaded,
        &kingmaker_core::transport::TransportRequest {
            expected_revision: 21,
            period: "next".into(),
            stage: kingmaker_core::transport::TransportStage::Arrivals,
        },
    )
    .unwrap();
    assert_eq!(
        loaded
            .account_by_address(&AccountAddress {
                owner_id: "seller".into(),
                currency_id: "COIN".into(),
                purpose: "operating_capital".into()
            })
            .unwrap()
            .balance,
        Money(10)
    );
    assert!(loaded
        .stockpiles
        .iter()
        .any(|s| s.owner_id == ExternalId::from("a")
            && s.quantity == 1.
            && s.purpose == "commercial"));
}
#[test]
fn current_market_order_proof_and_prepared_dispatch_reads_are_required() {
    let mut w = common::ready();
    w.market_data.phase_receipts[0].orders[0].quantity += 10.;
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_physical_supply(&mut w, &common::request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    for case in 0..3 {
        let mut w = common::ready();
        let prepared = prepare_physical_supply(&w, &common::request()).unwrap();
        match case {
            0 => w.accounts[0].balance = Money(0),
            1 => w.stockpiles[0].quantity += 1.,
            _ => w.transport_data.supply_routes[0]["capacity_per_week"] = json!(1),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(apply_physical_supply(&mut w, prepared).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn missing_routes_consumer_storage_or_money_are_real_blocked_conditions() {
    for case in 0..3 {
        let mut w = common::ready_with(|w| match case {
            0 => w.transport_data.supply_routes.clear(),
            1 => w.market_data.consumer_facility_by_province = None,
            _ => w.accounts[0].balance = Money(0),
        });
        let stocks = w.stockpiles.clone();
        let accounts = w.accounts.clone();
        let receipt = execute_physical_supply(&mut w, &common::request()).unwrap();
        assert!(w.mobility_data.shipments.is_empty());
        assert_eq!(w.stockpiles, stocks);
        assert_eq!(w.accounts, accounts);
        assert!(!receipt.diagnostics.is_empty());
    }
}
#[test]
fn ordinary_autonomous_choices_and_paid_operations_match_actual_js() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut w = common::ready();
    let mut snapshot = income_common::snapshot(&w);
    snapshot["campaign"]["campaign_id"] = json!(w.campaign.campaign_id);
    snapshot["registry"]["consumerFacilityByProvince"] =
        json!(w.market_data.consumer_facility_by_province);
    snapshot["registry"]["transportAssets"] = json!(w.transport_data.transport_assets);
    snapshot["registry"]["transportTerminals"] = json!(w.transport_data.transport_terminals);
    snapshot["registry"]["supplyRoutes"] = json!(w.transport_data.supply_routes);
    snapshot["registry"]["routeReservations"] = json!(w.transport_data.route_reservations);
    snapshot["appliedEventIds"] = json!(w
        .production_data
        .committed_operations
        .iter()
        .map(|o| &o.event_id)
        .collect::<Vec<_>>());
    let orders = &w.market_data.phase_receipts[0].orders;
    let ops = &w.market_data.phase_receipts[0].operations;
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
    child.stdin.take().unwrap().write_all(json!({"action":"physical","snapshot":snapshot,"period":"w","orders":orders,"marketOperations":ops}).to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let receipt = execute_physical_supply(&mut w, &common::request()).unwrap();
    let operations: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(expected["operations"].clone()).unwrap();
    assert_eq!(
        &receipt.operations[..receipt.operations.len() - 1],
        operations
    );
    assert_eq!(
        w.mobility_data.shipments[0]["id"],
        expected["registry"]["shipments"][0]["id"]
    );
    assert_eq!(
        w.mobility_data.shipments[0]["order_identity"],
        expected["registry"]["shipments"][0]["order_identity"]
    );
}
#[test]
fn automatic_two_edge_land_route_is_added_and_uses_per_edge_freight_and_dates() {
    let mut w = common::ready_with(|w| {
        w.pops[0].province_id = ProvinceId(3);
        w.production_data
            .storage_facilities
            .iter_mut()
            .find(|f| f.owner_id == ExternalId::from("a"))
            .unwrap()
            .province_id = ProvinceId(3);
        w.market_data.consumer_facility_by_province = Some(json!({"3":"wh:a"}));
        w.market_data
            .territory
            .iter_mut()
            .find(|p| p.province_id == ProvinceId(2))
            .unwrap()
            .extra
            .insert("neighbor_ids".into(), json!([1, 3]));
        w.market_data
            .territory
            .iter_mut()
            .find(|p| p.province_id == ProvinceId(3))
            .unwrap()
            .extra
            .insert("neighbor_ids".into(), json!([2]));
        w.transport_data.automatic_land_paths = Some(json!(true));
    });
    let receipt = execute_physical_supply(&mut w, &common::request()).unwrap();
    assert!(receipt.invariants.all_required_passed());
    assert_eq!(w.transport_data.supply_routes.len(), 2);
    let cargo = &w.mobility_data.shipments[0];
    assert_eq!(cargo["route_id"], json!("land-path:1:3"));
    assert_eq!(cargo["freight_paid_minor"], json!("4"));
    assert_eq!(
        cargo["arrival_date"],
        json!({"year":306,"month":1,"day":15})
    );
    assert_eq!(
        cargo["carrier_release_date"],
        json!({"year":306,"month":1,"day":29})
    );
}
#[test]
fn sequential_routes_share_terminal_capacity_and_do_not_allocate_it_twice() {
    let mut w = common::ready_with(|w| {
        w.market_data.needs.as_mut().unwrap().profiles[0].norms[0]
            .extra
            .insert("quantity_per_person_week".into(), json!(0.002));
        w.transport_data.transport_assets[0]["capacity"] = json!(10);
        w.transport_data.supply_routes[0]["capacity_per_week"] = json!(1);
        w.transport_data.supply_routes[0]["source_handling_per_week"] = json!(1);
        w.transport_data.supply_routes[0]["destination_handling_per_week"] = json!(1);
        w.transport_data.supply_routes[0]["id"] = json!("a");
        let mut other = w.transport_data.supply_routes[0].clone();
        other["id"] = json!("b");
        w.transport_data.supply_routes.push(other);
    });
    let receipt = execute_physical_supply(&mut w, &common::request()).unwrap();
    assert_eq!(w.mobility_data.shipments.len(), 1);
    assert_eq!(
        w.mobility_data.shipments[0]["shipped_quantity"].as_f64(),
        Some(1.)
    );
    assert!(receipt
        .diagnostics
        .iter()
        .any(|d| d["unfulfilled_quantity"].as_f64() == Some(1.)));
}
#[test]
fn earlier_route_wins_over_cheaper_slow_supply_and_invalid_fast_route_falls_back() {
    for blocked in [false, true] {
        let mut w = common::ready_with(|w| {
            let mut stock = w.stockpiles[0].clone();
            stock.id = 50;
            stock.external_id = "slow-iron".into();
            stock.province_id = ProvinceId(3);
            stock.facility_id = Some("wh:slow".into());
            w.stockpiles.push(stock);
            let mut warehouse = w
                .production_data
                .storage_facilities
                .iter()
                .find(|f| f.owner_id == ExternalId::from("seller"))
                .unwrap()
                .clone();
            warehouse.id = "wh:slow".into();
            warehouse.province_id = ProvinceId(3);
            w.production_data.storage_facilities.push(warehouse);
            let mut offer = w.market_data.sale_offers[0].clone();
            offer.id = Some("slow-offer".into());
            offer.stock_id = "slow-iron".into();
            offer
                .extra
                .insert("price_minor_per_unit".into(), json!("1"));
            w.market_data.sale_offers.push(offer);
            w.market_data
                .territory
                .iter_mut()
                .find(|p| p.province_id == ProvinceId(3))
                .unwrap()
                .extra
                .insert("neighbor_ids".into(), json!([2]));
            let mut asset = w.transport_data.transport_assets[0].clone();
            asset["id"] = json!("slow-cart");
            asset["home_province_id"] = json!(3);
            w.transport_data.transport_assets.push(asset);
            let mut route = w.transport_data.supply_routes[0].clone();
            route["id"] = json!("slow");
            route["asset_id"] = json!("slow-cart");
            route["from_province_id"] = json!(3);
            route["path"] = json!([3, 2]);
            route["travel_days"] = json!(14);
            w.transport_data.supply_routes.push(route);
            if blocked {
                w.transport_data.supply_routes[0]["currency_id"] = json!("OTHER");
            }
        });
        execute_physical_supply(&mut w, &common::request()).unwrap();
        assert_eq!(w.mobility_data.shipments.len(), 1);
        assert_eq!(
            w.mobility_data.shipments[0]["route_id"],
            json!(if blocked { "slow" } else { "route" })
        );
        assert_eq!(
            w.mobility_data.shipments[0]["goods_price_minor"],
            json!(if blocked { "1" } else { "10" })
        );
    }
}
