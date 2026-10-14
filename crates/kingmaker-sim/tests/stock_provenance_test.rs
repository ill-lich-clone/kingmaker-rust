#[path = "common/income.rs"]
mod income_common;
#[path = "common/labor.rs"]
mod labor_common;
#[path = "common/market.rs"]
mod market_common;
#[path = "common/production.rs"]
mod production_common;
use kingmaker_core::{
    income::StockContribution,
    mobility::{CohortSplitRequest, SplitTarget},
    *,
};
use serde_json::json;

#[test]
fn closing_income_preserves_creator_trace_of_a_real_industrial_input_purchase() {
    let mut w = production_common::world();
    w.accounts[0].balance = Money(1000);
    let method = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
    method.method_id = "factory:tools".into();
    method.output_goods_id = "tools".into();
    method.inputs = serde_json::from_value(json!([{"goods_id":"steel","amount":1}])).unwrap();
    let mut artisan = method.clone();
    artisan.kind = "artisan".into();
    artisan.method_id = "artisan:steel".into();
    artisan.output_goods_id = "steel".into();
    artisan.staffing = vec!["artisans".into()];
    artisan.inputs = serde_json::from_value(
        json!([{"goods_id":"iron","amount":1.1},{"goods_id":"coal","amount":0.4}]),
    )
    .unwrap();
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .push(artisan);
    w.production_data.factories[0].base.method_id = "factory:tools".into();
    w.production_data.catalog.as_mut().unwrap().goods.push(
        serde_json::from_value(json!({"goods_id":"tools","kind":"processed_good","unit":"t"}))
            .unwrap(),
    );
    w.production_data.storage_facilities[0].goods.insert(
        "tools".into(),
        serde_json::from_value(json!({"unit":"t","space_per_unit":1})).unwrap(),
    );
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"supplier-guild","owner_id":"supplier","province_id":1,"facility_id":"wh-supplier","method_id":"artisan:steel","people_slots":0,"assignments":[],"income_policy":{"source":"fixture","cash_reserve_minor":"0","payout_fraction":1}})).unwrap());
    let mut facility = w.production_data.storage_facilities[0].clone();
    facility.id = "wh-supplier".into();
    facility.owner_id = "supplier".into();
    w.production_data.storage_facilities.push(facility);
    let mut stock = w.stockpiles[0].clone();
    stock.id = 12;
    stock.external_id = "supplier-steel".into();
    stock.owner_id = "supplier".into();
    stock.goods = Goods::Steel;
    stock.quantity = 1.;
    stock.facility_id = Some("wh-supplier".into());
    stock.currency_id = Some("COIN".into());
    stock.inventory_origin = Some("produced".into());
    stock.method_kind = Some("artisan".into());
    w.stockpiles.push(stock);
    w.accounts.push(production_common::account(
        100,
        "supplier",
        "operating_capital",
        0,
    ));
    w.rebuild_account_index().unwrap();
    w.market_data = market_common::market_world().market_data;
    w.market_data.territory.truncate(1);
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("steel".into(), json!(3));
    w.market_data.needs=Some(serde_json::from_value(json!({"schema_version":"weekly-needs-v1","profileRefs":[{"pop_id":"p","need_profile_id":"need","size":1,"wealth_band_id":"poor","literacy":0.5}],"profiles":[{"need_profile_id":"need","species_food_factor":1,"norms":[]}],"resolver":{"period":"week","period_days":7}})).unwrap());
    let r = kingmaker_core::income::IncomeRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    };
    let basis = kingmaker_sim::income_basis::capture_income_opening(&w, &r).unwrap();
    income_common::opening_arrivals(&mut w, "w");
    kingmaker_sim::budget_cycle::execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    kingmaker_sim::production::execute_production_cycle(&mut w, &production_common::request())
        .unwrap();
    kingmaker_sim::market_cycle::execute_market_phase(
        &mut w,
        &kingmaker_core::market::MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: kingmaker_core::market::MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: None,
        },
    )
    .unwrap();
    assert_eq!(w.income_data.stock_contributions.len(), 1);
    income_common::finish_supply(&mut w, "w");
    kingmaker_sim::income_cycle::execute_income_closing(&mut w, &r, &basis).unwrap();
    assert_eq!(w.income_data.stock_contributions.len(), 1);
    let trace = &w.income_data.stock_contributions[0];
    assert_eq!(trace.quantity, 1.);
    assert_eq!(trace.producer_id, ExternalId::from("supplier-guild"));
    let bought = w
        .stockpiles
        .iter()
        .find(|s| s.external_id == trace.stock_id)
        .unwrap();
    assert_eq!(bought.owner_id, ExternalId::from("o"));
    assert_eq!(bought.inventory_origin.as_deref(), Some("purchased"));
}
fn artisan_world() -> WorldState {
    let mut w = production_common::world();
    let factory = w.production_data.factories.remove(0);
    let mut row = serde_json::to_value(factory).unwrap();
    row["id"] = json!("a");
    row["method_id"] = json!("artisan:steel");
    row["people_slots"] = json!(1);
    row["assignments"][0]
        .as_object_mut()
        .unwrap()
        .remove("contract_id");
    w.production_data
        .artisans
        .push(serde_json::from_value(row).unwrap());
    w.production_data.contracts.clear();
    let m = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
    m.kind = "artisan".into();
    m.method_id = "artisan:steel".into();
    m.staffing = vec!["artisans".into()];
    w.pops[0].pop_type_id = "artisans".into();
    w.pops[0].pop_type = "artisans".into();
    w.accounts[1].balance = Money(1000);
    w.refresh_enterprise_views().unwrap();
    w
}
#[test]
fn actual_production_records_creator_quantity_and_real_consumption_removes_it() {
    let mut w = artisan_world();
    kingmaker_sim::production::execute_production_cycle(&mut w, &production_common::request())
        .unwrap();
    assert_eq!(w.income_data.stock_contributions.len(), 1);
    assert_eq!(
        w.income_data.stock_contributions[0].producer_id,
        ExternalId::from("a")
    );
    assert_eq!(w.income_data.stock_contributions[0].quantity, 1.);
    w.accounts.push(production_common::account(
        100,
        "p",
        "personal_savings",
        10000,
    ));
    w.rebuild_account_index().unwrap();
    w.market_data = market_common::market_world().market_data;
    w.market_data.territory.truncate(1);
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("steel".into(), json!(3));
    w.market_data.needs=Some(serde_json::from_value(json!({"schema_version":"weekly-needs-v1","profileRefs":[{"pop_id":"p","need_profile_id":"need","size":1,"wealth_band_id":"poor","literacy":0.5}],"profiles":[{"need_profile_id":"need","species_food_factor":1,"norms":[{"need_id":"steel","goods_id":"steel","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":1,"unit":"t","allocation_rule":"none"}]}],"resolver":{"period":"week","period_days":7}})).unwrap());
    kingmaker_sim::market_cycle::execute_market_phase(
        &mut w,
        &kingmaker_core::market::MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: kingmaker_core::market::MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: None,
        },
    )
    .unwrap();
    assert!(w.income_data.stock_contributions.is_empty());
}
fn cohort_world(owner: &str) -> WorldState {
    let mut w = labor_common::world(0);
    let mut m = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    m.kind = "artisan".into();
    m.method_id = "artisan:steel".into();
    m.output_goods_id = "steel".into();
    m.inputs = serde_json::from_value(json!([{"goods_id":"iron","amount":1}])).unwrap();
    w.production_data.catalog.as_mut().unwrap().methods.push(m);
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"historical-artisan","owner_id":owner,"province_id":1,"method_id":"artisan:steel","people_slots":600,"assignments":[]})).unwrap());
    let mut stock = w.stockpiles[0].clone();
    stock.id = 900;
    stock.external_id = "historical-steel".into();
    stock.owner_id = "a".into();
    stock.goods = Goods::Steel;
    stock.quantity = 12.25;
    stock.inventory_origin = Some(
        if owner == "a" {
            "produced"
        } else {
            "purchased"
        }
        .into(),
    );
    w.stockpiles.push(stock);
    w.income_data.stock_contributions = vec![StockContribution {
        stock_id: "historical-steel".into(),
        producer_id: "historical-artisan".into(),
        quantity: 12.25,
    }];
    w
}
#[test]
fn full_actual_cohort_claim_preserves_stock_creator_trace_and_owner_rights() {
    let mut w = cohort_world("a");
    let receipt = kingmaker_sim::cohort_cycle::execute_pop_split(
        &mut w,
        &CohortSplitRequest {
            expected_revision: 20,
            event_id: "full-claim".into(),
            pop_id: "a".into(),
            workforce_count: 600,
            target: SplitTarget::default(),
        },
    )
    .unwrap();
    let child = receipt.child_id.unwrap();
    let trace = &w.income_data.stock_contributions;
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0].quantity, 12.25);
    let stock = w
        .stockpiles
        .iter()
        .find(|s| s.external_id == trace[0].stock_id)
        .unwrap();
    assert_eq!(stock.owner_id, child);
    assert_eq!(stock.quantity, trace[0].quantity);
    assert_eq!(w.production_data.artisans[0].base.owner_id, Some(child));
    let restored: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    assert_eq!(
        restored.income_data.stock_contributions,
        w.income_data.stock_contributions
    );
}
#[test]
fn partial_actual_cohort_claim_splits_historical_trace_proportionally_without_new_goods() {
    let mut w = cohort_world("original-creator");
    let receipt = kingmaker_sim::cohort_cycle::execute_pop_split(
        &mut w,
        &CohortSplitRequest {
            expected_revision: 20,
            event_id: "partial-claim".into(),
            pop_id: "a".into(),
            workforce_count: 60,
            target: SplitTarget::default(),
        },
    )
    .unwrap();
    let child = receipt.child_id.unwrap();
    assert_eq!(w.income_data.stock_contributions.len(), 2);
    let total: f64 = w
        .income_data
        .stock_contributions
        .iter()
        .map(|c| c.quantity)
        .sum();
    assert!((total - 12.25).abs() < 1e-9);
    let transferred = w
        .income_data
        .stock_contributions
        .iter()
        .find(|c| {
            w.stockpiles
                .iter()
                .any(|s| s.external_id == c.stock_id && s.owner_id == child)
        })
        .unwrap();
    assert!((transferred.quantity - 1.225).abs() < 1e-9);
}
