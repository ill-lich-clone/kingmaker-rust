#![allow(dead_code)]
#[path = "production.rs"]
mod production;
use kingmaker_core::{transport::*, *};
use serde_json::json;
pub fn request(period: &str) -> TransportRequest {
    TransportRequest {
        expected_revision: 20,
        period: period.into(),
        stage: TransportStage::Arrivals,
    }
}
/// A declared miniature shipment with real cargo, paid escrow and a destination reservation.
pub fn world(capacity: f64) -> WorldState {
    let mut w = production::world();
    w.campaign.day = 8;
    w.stockpiles.clear();
    w.production_data.factories.clear();
    w.production_data.contracts.clear();
    w.provinces.push(ProvinceRecord {
        id: ProvinceId(2),
        ..Default::default()
    });
    w.accounts
        .push(production::account(3, "shipment:cargo", "trade_escrow", 1));
    w.accounts
        .push(production::account(4, "seller", "operating_capital", 0));
    w.accounts
        .push(production::account(5, "buyer", "personal_savings", 0));
    w.rebuild_account_index().unwrap();
    w.production_data.storage_facilities=serde_json::from_value(json!([{"id":"dest","owner_id":"buyer","province_id":2,"capacity":capacity,"unit":"space","status":"source","source":"fixture","goods":{"iron":{"unit":"t","space_per_unit":1}}}])).unwrap();
    w.production_data.storage_reservations=serde_json::from_value(json!([{"id":"shipment:cargo:storage","owner_id":"buyer","facility_id":"dest","goods_id":"iron","unit":"t","status":"held","quantity":3,"accepted_quantity":3,"period":"prior","fee_paid_minor":"0"}])).unwrap();
    w.mobility_data.shipments = vec![
        json!({"id":"cargo","buyer_id":"buyer","buyer_purpose":"personal_savings","seller_id":"seller","seller_purpose":"operating_capital","stock_id":"source-iron","goods_id":"iron","unit":"t","currency_id":"COIN","route_id":"route","asset_id":"cart","carrier_id":"carrier","from_province_id":1,"to_province_id":2,"facility_id":"dest","storage_reservation_id":"shipment:cargo:storage","shipped_quantity":3,"remaining_quantity":3,"arrived_quantity":0,"space_per_unit":1,"goods_price_minor":"1","paid_to_seller_minor":"0","freight_paid_minor":"0","departed_date":{"year":306,"month":1,"day":1},"arrival_date":{"year":306,"month":1,"day":8},"carrier_release_date":{"year":306,"month":1,"day":10},"return_days":2,"status":"in_transit","goods_origin":"opening","arrival_receipts":[],"unknown_author_field":{"keep":true}}),
    ];
    w.refresh_enterprise_views().unwrap();
    w
}
pub fn cash(w: &WorldState, owner: &str, purpose: &str) -> i64 {
    w.accounts
        .iter()
        .filter(|a| a.owner_id == ExternalId::from(owner) && a.purpose == purpose)
        .map(|a| a.balance.0)
        .sum()
}
