#![allow(dead_code)]
#[path = "market.rs"]
mod market;
#[path = "production.rs"]
mod production;
use kingmaker_core::{week::*, *};
use serde_json::json;
pub fn empty() -> WorldState {
    let mut w = WorldState::default();
    w.campaign = CampaignHeader {
        campaign_id: "week-fixture".into(),
        revision: 20,
        year: 306,
        month: 1,
        day: 1,
        ..Default::default()
    };
    w.production_data.catalog = Some(
        serde_json::from_value(
            json!({"version":"production-foundation-v1","goods":[],"methods":[]}),
        )
        .unwrap(),
    );
    w.market_data.needs=Some(serde_json::from_value(json!({"schema_version":"weekly-needs-v1","profileRefs":[],"profiles":[],"resolver":{"period":"week","period_days":7}})).unwrap());
    w.mobility_data.reference = Some(json!({"professions":[]}));
    w
}
pub fn request(w: &WorldState) -> NativeWeekRequest {
    NativeWeekRequest {
        expected_revision: w.campaign.revision,
        period: format!("week-{}", w.campaign.revision),
        policies: Default::default(),
        market_profile: Default::default(),
        market_priorities: None,
        production_profile: Default::default(),
    }
}
pub fn factory(remote: bool) -> WorldState {
    let mut w = production::world();
    w.campaign.campaign_id = "real-week".into();
    w.market_data = market::market_world().market_data;
    w.market_data.territory.truncate(if remote { 2 } else { 1 });
    w.market_data.parameters.as_mut().unwrap().instant_food = Some(true);
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("steel".into(), json!(3));
    w.market_data.needs=Some(serde_json::from_value(json!({"schema_version":"weekly-needs-v1","profileRefs":[{"pop_id":if remote {"buyer"}else{"p"},"need_profile_id":"equipment","size":1,"wealth_band_id":"poor","literacy":0.5}],"profiles":[{"need_profile_id":"equipment","species_food_factor":1,"norms":[{"need_id":"equipment","goods_id":"steel","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":1,"unit":"t","allocation_rule":"none"}]}],"resolver":{"period":"week","period_days":7}})).unwrap());
    w.mobility_data.reference = Some(json!({"professions":[]}));
    let mapping = w.production_data.storage_facilities[0].goods["steel"].clone();
    w.production_data.storage_facilities[0]
        .goods
        .insert("iron".into(), mapping.clone());
    w.production_data.storage_facilities[0]
        .goods
        .insert("coal".into(), mapping);
    if remote {
        w.provinces.push(ProvinceRecord {
            id: ProvinceId(2),
            owner_state_id: Some("s".into()),
            pop_range: (1, 2),
            ..Default::default()
        });
        let mut buyer = w.pops[0].clone();
        buyer.id = PopId(2);
        buyer.external_id = "buyer".into();
        buyer.province_id = ProvinceId(2);
        buyer.workforce = 0.;
        buyer.employed = 0.;
        buyer.pop_type_id = "farmers".into();
        buyer.pop_type = "farmers".into();
        w.pops.push(buyer);
        w.accounts
            .push(production::account(100, "buyer", "personal_savings", 500));
        w.rebuild_account_index().unwrap();
        w.market_data.territory[0]
            .extra
            .insert("neighbor_ids".into(), json!([2]));
        w.market_data.territory[1]
            .extra
            .insert("neighbor_ids".into(), json!([1]));
        w.market_data.territory[1].region_id = Some("other".into());
        w.production_data.storage_facilities.push(serde_json::from_value(json!({"id":"buyer-wh","owner_id":"buyer","province_id":2,"capacity":100,"unit":"space","status":"source","source":"fixture","goods":{"steel":{"unit":"t","space_per_unit":1}}})).unwrap());
        w.market_data.consumer_facility_by_province = Some(json!({"2":"buyer-wh"}));
        w.transport_data.transport_assets = vec![
            json!({"id":"cart","owner_id":"carrier","home_province_id":1,"modes":["land"],"unit":"space","capacity":2,"status":"source","source":"fixture"}),
        ];
        w.transport_data.supply_routes = vec![
            json!({"id":"road","from_province_id":1,"to_province_id":2,"asset_id":"cart","carrier_id":"carrier","mode":"land","unit":"space","currency_id":"COIN","capacity_per_week":2,"source_handling_per_week":2,"destination_handling_per_week":2,"travel_days":7,"return_days":7,"freight_minor_per_space":"2","goods":{"steel":{"unit":"t","space_per_unit":1}},"path":[1,2],"status":"source","source":"fixture"}),
        ];
    }
    w
}
