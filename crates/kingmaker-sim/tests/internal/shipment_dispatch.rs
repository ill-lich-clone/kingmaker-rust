use crate::{
    operation_session::apply_prepared,
    shipment_dispatch::{DispatchSession, ShipmentOrder},
};
use kingmaker_core::*;
use serde_json::json;
#[path = "../common/labor.rs"]
mod labor_common;
pub(crate) fn world() -> WorldState {
    let mut w = WorldState::default();
    w.campaign = CampaignHeader {
        campaign_id: "dispatch-fixture".into(),
        revision: 20,
        year: 306,
        month: 1,
        day: 1,
        ..Default::default()
    };
    for id in 1..=2 {
        w.provinces.push(ProvinceRecord {
            id: ProvinceId(id),
            ..Default::default()
        });
        w.market_data.territory.push(serde_json::from_value(json!({"province_id":id,"region_id":format!("r{id}"),"is_floating":false,"landmass_id":"land","neighbor_ids":[if id==1{2}else{1}]})).unwrap());
    }
    w.accounts=serde_json::from_value(json!([{"id":1,"external_id":"buyer-cash","owner_id":"buyer","currency_id":"COIN","purpose":"personal_savings","balance":"30"},{"id":2,"external_id":"seller-cash","owner_id":"seller","currency_id":"COIN","purpose":"operating_capital","balance":"0"}])).unwrap();
    w.rebuild_account_index().unwrap();
    w.stockpiles.push(StockpileRecord {
        id: 10,
        external_id: "iron".into(),
        province_id: ProvinceId(1),
        owner_id: "seller".into(),
        goods: Goods::Iron,
        quantity: 5.,
        unit: Some(json!("t")),
        currency_id: Some("COIN".into()),
        inventory_origin: Some("opening".into()),
        ..Default::default()
    });
    w.market_data.sale_offers=serde_json::from_value(json!([{"id":"offer","stock_id":"iron","seller_id":"seller","seller_purpose":"operating_capital","goods_id":"iron","currency_id":"COIN","unit":"t","price_minor_per_unit":"10","status":"source","source":"fixture"}])).unwrap();
    w.transport_data.transport_assets = vec![
        json!({"id":"cart","owner_id":"carrier","home_province_id":1,"modes":["land"],"unit":"space","capacity":2,"status":"source","source":"fixture"}),
    ];
    w.transport_data.supply_routes = vec![
        json!({"id":"route","from_province_id":1,"to_province_id":2,"asset_id":"cart","carrier_id":"carrier","mode":"land","unit":"space","currency_id":"COIN","capacity_per_week":3,"source_handling_per_week":3,"destination_handling_per_week":3,"travel_days":7,"return_days":7,"freight_minor_per_space":"2","goods":{"iron":{"unit":"t","space_per_unit":1}},"path":[1,2],"status":"source","source":"fixture"}),
    ];
    w.production_data.storage_facilities=serde_json::from_value(json!([{"id":"dest","owner_id":"buyer","province_id":2,"capacity":10,"unit":"space","status":"source","source":"fixture","goods":{"iron":{"unit":"t","space_per_unit":1}}}])).unwrap();
    w
}
fn order(id: &str, quantity: f64) -> ShipmentOrder {
    ShipmentOrder {
        id: id.into(),
        buyer_id: "buyer".into(),
        purpose: "personal_savings".into(),
        offer_id: "offer".into(),
        route_id: "route".into(),
        destination_facility_id: "dest".into(),
        quantity,
        inventory_purpose: None,
        pop_id: None,
    }
}
#[test]
fn real_dispatch_moves_goods_to_cargo_and_cash_to_escrow_and_carrier() {
    let mut w = world();
    let mut session = DispatchSession::new(&w, "w").unwrap();
    let outcome = session.dispatch(&order("order", 5.)).unwrap();
    assert_eq!(outcome.quantity, 2.);
    let plan = session.finish().unwrap();
    let cargo = plan.shipments[0].clone();
    apply_prepared(&mut w, plan.changes).unwrap();
    w.mobility_data.shipments = plan.shipments;
    w.transport_data.route_reservations = plan.route_reservations;
    assert_eq!(w.stockpiles[0].quantity, 3.);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), 30);
    assert_eq!(
        w.account_by_address(&AccountAddress {
            owner_id: "shipment:order".into(),
            currency_id: "COIN".into(),
            purpose: "trade_escrow".into()
        })
        .unwrap()
        .balance,
        Money(20)
    );
    assert_eq!(
        w.account_by_address(&AccountAddress {
            owner_id: "carrier".into(),
            currency_id: "COIN".into(),
            purpose: "operating_capital".into()
        })
        .unwrap()
        .balance,
        Money(4)
    );
    assert_eq!(cargo["arrival_date"], json!({"year":306,"month":1,"day":8}));
    assert_eq!(
        cargo["carrier_release_date"],
        json!({"year":306,"month":1,"day":15})
    );
    assert_eq!(cargo["paid_to_seller_minor"], json!("0"));
}
#[test]
fn shared_fleet_route_terminals_reservations_and_cash_are_not_spent_twice() {
    for case in 0..4 {
        let mut w = world();
        w.transport_data.transport_assets[0]["capacity"] = json!(10);
        match case {
            0 => w.transport_data.supply_routes[0]["capacity_per_week"] = json!(1),
            1 => w.transport_data.supply_routes[0]["source_handling_per_week"] = json!(1),
            2 => w.transport_data.supply_routes[0]["destination_handling_per_week"] = json!(1),
            _ => w.accounts[0].balance = Money(12),
        };
        let mut session = DispatchSession::new(&w, "w").unwrap();
        assert_eq!(session.dispatch(&order("first", 1.)).unwrap().quantity, 1.);
        assert_eq!(session.dispatch(&order("second", 1.)).unwrap().quantity, 0.);
        let plan = session.finish().unwrap();
        assert_eq!(plan.shipments.len(), 1);
    }
}
#[test]
fn held_stock_and_existing_carrier_return_date_keep_their_capacity() {
    let mut w = world();
    w.production_data.goods_reservations =
        serde_json::from_value(json!([{"stock_id":"iron","quantity":4,"status":"held"}])).unwrap();
    let mut session = DispatchSession::new(&w, "w").unwrap();
    assert_eq!(session.dispatch(&order("first", 5.)).unwrap().quantity, 1.);
    let mut w = world();
    w.mobility_data.shipments.push(json!({"id":"old","asset_id":"cart","remaining_quantity":0,"shipped_quantity":2,"space_per_unit":1,"carrier_release_date":{"year":306,"month":1,"day":8}}));
    let mut session = DispatchSession::new(&w, "w").unwrap();
    assert_eq!(session.dispatch(&order("first", 1.)).unwrap().quantity, 0.);
}
#[test]
fn late_negative_quote_or_wrong_destination_never_publishes_an_earlier_dispatch() {
    let mut w = world();
    let mut bad = w.market_data.sale_offers[0].clone();
    bad.id = Some("bad".into());
    bad.extra.insert("price_minor_per_unit".into(), json!("-1"));
    w.market_data.sale_offers.push(bad);
    let before = serde_json::to_value(&w).unwrap();
    let mut session = DispatchSession::new(&w, "w").unwrap();
    session.dispatch(&order("first", 1.)).unwrap();
    let mut bad = order("bad", 1.);
    bad.offer_id = "bad".into();
    assert!(session.dispatch(&bad).is_err());
    assert!(session.finish().is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn ordinary_lease_fractional_quote_scarce_cash_and_capacity_dispatch_match_js() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for case in 0..4 {
        let mut w = world();
        match case {
            1 => {
                w.production_data.storage_facilities[0].owner_id = "landlord".into();
                w.production_data.storage_facilities[0].access_contracts=serde_json::from_value(json!([{"owner_id":"buyer","currency_id":"COIN","purpose":"personal_savings","price_minor_per_space":"1","status":"source","source":"fixture","cancellation_policy":"refund_unused"}])).unwrap();
            }
            2 => w.market_data.sale_offers[0]
                .extra
                .insert(
                    "price_minor_rate".into(),
                    json!({"numerator":"21","denominator":"2"}),
                )
                .map(|_| ())
                .unwrap_or(()),
            3 => w.accounts[0].balance = Money(5),
            _ => (),
        };
        let wanted = order("order", 5.);
        let mut snapshot = labor_common::snapshot(&w);
        snapshot["campaign"] = json!({"campaign_id":w.campaign.campaign_id,"date":{"year":w.campaign.year,"month":w.campaign.month,"day":w.campaign.day}});
        snapshot["registry"]["transportAssets"] = json!(w.transport_data.transport_assets);
        snapshot["registry"]["transportTerminals"] = json!(w.transport_data.transport_terminals);
        snapshot["registry"]["supplyRoutes"] = json!(w.transport_data.supply_routes);
        snapshot["registry"]["routeReservations"] = json!(w.transport_data.route_reservations);
        snapshot["registry"]["shipments"] = json!(w.mobility_data.shipments);
        snapshot["registry"]["territory"] = json!(w.market_data.territory);
        snapshot["registry"]["saleOffers"] = json!(w.market_data.sale_offers);
        snapshot["registry"]["storageReservations"] = json!(w.production_data.storage_reservations);
        snapshot["appliedEventIds"] = json!([]);
        for (row, stock) in snapshot["stocks"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(&w.stockpiles)
        {
            row["inventory_origin"] = json!(stock.inventory_origin);
        }
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
                json!({"action":"dispatch","snapshot":snapshot,"period":"w","orders":[wanted]})
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
        let mut session = DispatchSession::new(&w, "w").unwrap();
        let accepted = session.dispatch(&wanted).unwrap();
        let plan = session.finish().unwrap();
        let ops: Vec<kingmaker_core::operations::EconomicOperation> =
            serde_json::from_value(expected["operations"].clone()).unwrap();
        assert_eq!(plan.operations, ops, "case{case}");
        let expected_qty = expected["registry"]["shipments"][0]["shipped_quantity"]
            .as_f64()
            .unwrap();
        assert!((accepted.quantity - expected_qty).abs() < 1e-12);
        let mut actual = plan.shipments;
        let mut expected = expected["registry"]["shipments"].clone();
        for row in &mut actual {
            for key in [
                "shipped_quantity",
                "remaining_quantity",
                "arrived_quantity",
                "space_per_unit",
            ] {
                row[key] = json!(row[key].as_f64().unwrap());
            }
        }
        for row in expected.as_array_mut().unwrap() {
            for key in [
                "shipped_quantity",
                "remaining_quantity",
                "arrived_quantity",
                "space_per_unit",
            ] {
                row[key] = json!(row[key].as_f64().unwrap());
            }
        }
        assert_eq!(json!(actual), expected, "case{case}");
    }
}
