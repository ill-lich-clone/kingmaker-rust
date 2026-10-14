#![allow(dead_code)]
#[path = "market.rs"]
mod market;
use kingmaker_core::{market::*, production::*, transport::*, *};
use serde_json::json;
pub fn request() -> TransportRequest {
    TransportRequest {
        expected_revision: 20,
        period: "w".into(),
        stage: TransportStage::Dispatch,
    }
}
/// Author miniature: one consumer's real weekly iron need, foreign stock/offer and paid route.
pub fn opening() -> WorldState {
    let mut w = market::market_world();
    w.pops.truncate(1);
    w.pops[0].province_id = ProvinceId(2);
    w.accounts.retain(|a| a.owner_id != ExternalId::from("b"));
    w.accounts[0].balance = Money(30);
    w.accounts[1].purpose = "operating_capital".into();
    w.rebuild_account_index().unwrap();
    for p in &mut w.provinces {
        p.pop_range = if p.id == ProvinceId(2) {
            (0, 1)
        } else {
            (0, 0)
        };
    }
    w.market_data.needs=Some(serde_json::from_value(json!({"schema_version":"weekly-needs-v1","profileRefs":[{"pop_id":"a","need_profile_id":"equipment","size":1000,"wealth_band_id":"poor","literacy":0.5}],"profiles":[{"need_profile_id":"equipment","species_food_factor":1,"norms":[{"need_id":"equipment","goods_id":"iron","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":0.001,"unit":"t","allocation_rule":"none"}]}],"resolver":{"period":"week","period_days":7}})).unwrap());
    w.market_data.parameters.as_mut().unwrap().instant_food = Some(false);
    w.market_data.consumer_facility_by_province = Some(json!({"2":"wh:a"}));
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("iron".into(), json!(0.001));
    for p in &mut w.market_data.territory {
        p.region_id = Some(format!("r{}", p.province_id.0).into());
        p.extra.insert(
            "neighbor_ids".into(),
            json!(if p.province_id == ProvinceId(1) {
                vec![2]
            } else {
                vec![1]
            }),
        );
    }
    w.stockpiles[0].external_id = "iron".into();
    w.stockpiles[0].goods = Goods::Iron;
    w.stockpiles[0].quantity = 5.;
    w.stockpiles[0].province_id = ProvinceId(1);
    for f in &mut w.production_data.storage_facilities {
        let mapping = f.goods.get("grain").unwrap().clone();
        f.goods.clear();
        f.goods.insert("iron".into(), mapping);
        if f.owner_id == ExternalId::from("a") {
            f.province_id = ProvinceId(2);
        } else if f.owner_id == ExternalId::from("seller") {
            f.province_id = ProvinceId(1);
        }
    }
    w.production_data.catalog.as_mut().unwrap().goods.push(
        serde_json::from_value(json!({"goods_id":"iron","kind":"raw_resource","unit":"t"}))
            .unwrap(),
    );
    w.market_data.sale_offers=serde_json::from_value(json!([{"id":"offer","stock_id":"iron","seller_id":"seller","seller_purpose":"operating_capital","goods_id":"iron","currency_id":"COIN","unit":"t","price_minor_per_unit":"10","status":"source","source":"fixture"}])).unwrap();
    w.transport_data.transport_assets = vec![
        json!({"id":"cart","owner_id":"carrier","home_province_id":1,"modes":["land"],"unit":"space","capacity":2,"status":"source","source":"fixture"}),
    ];
    w.transport_data.supply_routes = vec![
        json!({"id":"route","from_province_id":1,"to_province_id":2,"asset_id":"cart","carrier_id":"carrier","mode":"land","unit":"space","currency_id":"COIN","capacity_per_week":3,"source_handling_per_week":3,"destination_handling_per_week":3,"travel_days":7,"return_days":7,"freight_minor_per_space":"2","goods":{"iron":{"unit":"t","space_per_unit":1}},"path":[1,2],"status":"source","source":"fixture"}),
    ];
    w
}
pub fn ready() -> WorldState {
    ready_with(|_| {})
}
pub fn ready_with(change: impl FnOnce(&mut WorldState)) -> WorldState {
    let mut w = ready_through_market(change);
    kingmaker_sim::local_food_reserves::execute_local_food_reserves(
        &mut w,
        &TransportRequest {
            stage: TransportStage::LocalFoodReserves,
            ..request()
        },
    )
    .unwrap();
    w
}
pub fn ready_through_market(change: impl FnOnce(&mut WorldState)) -> WorldState {
    let mut w = opening();
    change(&mut w);
    kingmaker_sim::shipment_arrivals::execute_shipment_arrivals(
        &mut w,
        &TransportRequest {
            stage: TransportStage::Arrivals,
            ..request()
        },
    )
    .unwrap();
    kingmaker_sim::budget_cycle::execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    kingmaker_sim::production::execute_production_cycle(
        &mut w,
        &ProductionRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
            profile: Default::default(),
            maintenance_receipts: vec![],
        },
    )
    .unwrap();
    kingmaker_sim::market_cycle::execute_market_phase(
        &mut w,
        &MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: None,
        },
    )
    .unwrap();
    w
}
