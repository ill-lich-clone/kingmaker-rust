#![allow(dead_code)]
use kingmaker_core::production::*;
use kingmaker_core::*;
use serde_json::json;

pub fn account(id: u32, owner: &str, purpose: &str, amount: i64) -> AccountBalance {
    AccountBalance {
        id: AccountId(id),
        external_id: format!("cash:{id}").into(),
        owner_id: owner.into(),
        currency_id: "COIN".into(),
        purpose: purpose.into(),
        balance: Money(amount),
    }
}
pub fn world() -> WorldState {
    let mut w = WorldState::default();
    w.campaign = CampaignHeader {
        campaign_id: "fixture".into(),
        revision: 20,
        year: 306,
        month: 1,
        day: 1,
        ..Default::default()
    };
    w.provinces.push(ProvinceRecord {
        id: ProvinceId(1),
        owner_state_id: Some("s".into()),
        pop_range: (0, 1),
        ..Default::default()
    });
    w.pops.push(PopRecord {
        id: PopId(1),
        external_id: "p".into(),
        state_id: Some("s".into()),
        province_id: ProvinceId(1),
        pop_type: "craftsmen".into(),
        pop_type_id: "craftsmen".into(),
        currency_id: Some("COIN".into()),
        size: 1,
        workforce: 1.0,
        employed: 1.0,
        ..Default::default()
    });
    w.accounts = vec![
        account(1, "o", "employer_operating_capital", 20),
        account(2, "o", "operating_capital", 0),
    ];
    w.production_data.catalog=Some(serde_json::from_value(json!({"version":"production-foundation-v1","goods":[{"goods_id":"iron","kind":"raw_resource","unit":"t"},{"goods_id":"coal","kind":"raw_resource","unit":"t"},{"goods_id":"steel","kind":"processed_good","unit":"t"}],"methods":[{"method_id":"factory:steel","kind":"factory","status":"ready","output_goods_id":"steel","output_unit":"t","weekly_productivity":1,"staffing":["craftsmen","clerks"],"technology":[],"inputs":[{"goods_id":"iron","amount":1.1},{"goods_id":"coal","amount":0.4}]}]})).unwrap());
    w.production_data.factories.push(serde_json::from_value(json!({"id":"f","owner_id":"o","province_id":1,"region_id":"r","method_id":"factory:steel","facility_id":"wh","status":"operating","people_slots":40000,"capacity_source":"fixture","equipment_capacity_weekly":100,"equipment_source":"fixture","staff_count":1,"tenure_days":0,"parameters":{"source":"fixture","status":"source","tenure_Bmax":0,"tenure_Tsat":30},"assignments":[{"pop_id":"p","workers_count":1,"time_fraction":1,"employment_id":"e","contract_id":"c"}],"implemented_technologies":[]})).unwrap());
    w.production_data.contracts.push(serde_json::from_value(json!({"contract_id":"c","employer_entity_id":"o","currency_id":"COIN","weekly_wage_rate":{"numerator":"10","denominator":"1"},"guaranteed_idle_weekly_rate":{"numerator":"2","denominator":"1"},"expectedPayroll":{"tax_rate":0.1}})).unwrap());
    w.production_data.storage_facilities.push(serde_json::from_value(json!({"id":"wh","owner_id":"o","province_id":1,"capacity":100,"unit":"slots","status":"approved","source":"fixture","approval_ref":"fixture","goods":{"steel":{"unit":"t","space_per_unit":1}}})).unwrap());
    w.stockpiles = vec![
        StockpileRecord {
            id: 10,
            external_id: "iron".into(),
            province_id: ProvinceId(1),
            owner_id: "o".into(),
            goods: Goods::Iron,
            quantity: 1.1,
            unit: Some(json!("t")),
            ..Default::default()
        },
        StockpileRecord {
            id: 11,
            external_id: "coal".into(),
            province_id: ProvinceId(1),
            owner_id: "o".into(),
            goods: Goods::Coal,
            quantity: 0.4,
            unit: Some(json!("t")),
            ..Default::default()
        },
    ];
    w.rebuild_account_index().unwrap();
    w.refresh_enterprise_views().unwrap();
    w
}
pub fn request() -> ProductionRequest {
    ProductionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: ProductionPolicies::default(),
        profile: ProductionProfile::default(),
        maintenance_receipts: vec![],
    }
}
