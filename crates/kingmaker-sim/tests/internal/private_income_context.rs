#[path = "../common/market.rs"]
mod market;
#[path = "../common/production.rs"]
mod production;
use kingmaker_core::{
    income::{IncomeRequest, StockContribution},
    *,
};
use serde_json::json;
#[test]
fn private_next_period_after_method_change_preserves_prior_goods_income() {
    let mut w = production::world();
    w.production_data.factories.clear();
    w.production_data.contracts.clear();
    w.pops[0].pop_type_id = "artisans".into();
    w.pops[0].pop_type = "artisans".into();
    w.accounts[1].balance = Money(1000);
    let method = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
    method.method_id = "artisan:tools".into();
    method.kind = "artisan".into();
    method.output_goods_id = "tools".into();
    method.staffing = vec!["artisans".into()];
    w.production_data.catalog.as_mut().unwrap().goods.push(
        serde_json::from_value(json!({"goods_id":"tools","kind":"processed_good","unit":"t"}))
            .unwrap(),
    );
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"a","owner_id":"o","province_id":1,"facility_id":"wh","method_id":"artisan:tools","people_slots":1,"assignments":[{"pop_id":"p","workers_count":1,"time_fraction":1,"employment_id":"e"}],"income_policy":{"source":"fixture","cash_reserve_minor":"0","payout_fraction":1}})).unwrap());
    w.production_data.storage_facilities[0].goods.insert(
        "tools".into(),
        serde_json::from_value(json!({"unit":"t","space_per_unit":1})).unwrap(),
    );
    let mut stock = w.stockpiles[0].clone();
    stock.id = 12;
    stock.external_id = "old-output".into();
    stock.goods = Goods::Steel;
    stock.quantity = 1.;
    stock.currency_id = Some("COIN".into());
    stock.inventory_origin = Some("produced".into());
    stock.method_kind = Some("artisan".into());
    stock.facility_id = Some("wh".into());
    w.stockpiles.push(stock);
    w.income_data.stock_contributions = vec![StockContribution {
        stock_id: "old-output".into(),
        producer_id: "a".into(),
        quantity: 1.,
    }];
    w.accounts
        .push(production::account(100, "p", "personal_savings", 10000));
    w.rebuild_account_index().unwrap();
    w.market_data = market::market_world().market_data;
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
    let mut state = w.clone();
    let r = IncomeRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    };
    let basis = crate::income_basis::capture_income_opening(&state, &r).unwrap();
    let supply_request = kingmaker_core::transport::TransportRequest {
        expected_revision: 20,
        period: "w".into(),
        stage: kingmaker_core::transport::TransportStage::Arrivals,
    };
    crate::shipment_arrivals::execute_shipment_arrivals(&mut state, &supply_request).unwrap();
    crate::budget_cycle::execute_budget_cycle(
        &mut state,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    crate::production::execute_production_cycle(&mut state, &production::request()).unwrap();
    crate::market_cycle::execute_market_phase(
        &mut state,
        &kingmaker_core::market::MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: kingmaker_core::market::MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: None,
        },
    )
    .unwrap();
    crate::local_food_reserves::execute_local_food_reserves(
        &mut state,
        &kingmaker_core::transport::TransportRequest {
            stage: kingmaker_core::transport::TransportStage::LocalFoodReserves,
            ..supply_request.clone()
        },
    )
    .unwrap();
    crate::physical_supply::execute_physical_supply(
        &mut state,
        &kingmaker_core::transport::TransportRequest {
            stage: kingmaker_core::transport::TransportStage::Dispatch,
            ..supply_request
        },
    )
    .unwrap();
    crate::raw_procurement::execute_raw_procurement(
        &mut state,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    crate::income_cycle::execute_income_closing(&mut state, &r, &basis).unwrap();
    assert_eq!(
        state.production_data.artisans[0].base.extra["income_history"]
            ["taxable_distribution_minor"],
        json!("150")
    );
}

