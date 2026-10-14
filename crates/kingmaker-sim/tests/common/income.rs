#![allow(dead_code)]
#[path = "labor.rs"]
mod labor;
#[path = "market.rs"]
mod market;
#[path = "production.rs"]
mod production;
use kingmaker_core::{income::IncomeRequest, market::*, *};
use kingmaker_sim::{
    budget_cycle::execute_budget_cycle,
    income_basis::{capture_income_opening, IncomeOpeningBasis},
    market_cycle::execute_market_phase,
    production::execute_production_cycle,
};
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

pub fn request() -> IncomeRequest {
    IncomeRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    }
}
pub fn snapshot(w: &WorldState) -> Value {
    let mut s = labor::snapshot(w);
    s["registry"]["producerIncome"] = json!(w.income_data.producer_income);
    s["registry"]["shipments"] = json!(w.mobility_data.shipments);
    s["registry"]["storageReservations"] = json!(w.production_data.storage_reservations);
    s["reservations"] = json!(w.production_data.money_reservations);
    s["campaign"] =
        json!({"date":{"year":w.campaign.year,"month":w.campaign.month,"day":w.campaign.day}});
    s["registry"]["territory"] = json!(w.market_data.territory);
    s["registry"]["foundationParameters"] = json!(w.market_data.parameters);
    s["registry"]["saleOffers"] = json!(w.market_data.sale_offers);
    for (row, stock) in s["stocks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(&w.stockpiles)
    {
        row["currency_id"] = json!(stock.currency_id);
        row["inventory_origin"] = json!(stock.inventory_origin);
        row["method_kind"] = json!(stock.method_kind);
        row["inventory_purpose"] = json!(stock.purpose);
    }
    s
}
pub fn fixture() -> (WorldState, IncomeOpeningBasis, Value) {
    fixture_with_rent(false)
}
pub fn fixture_with_rent(rent: bool) -> (WorldState, IncomeOpeningBasis, Value) {
    fixture_configure(rent, |_| {})
}
pub fn fixture_configure(
    rent: bool,
    configure: impl FnOnce(&mut WorldState),
) -> (WorldState, IncomeOpeningBasis, Value) {
    let mut w = production::world();
    if rent {
        w.production_data.storage_facilities[0].owner_id = "landlord".into();
        w.production_data.storage_facilities[0].access_contracts=vec![serde_json::from_value(json!({"owner_id":"o","currency_id":"COIN","purpose":"employer_operating_capital","price_minor_per_space":"5","cancellation_policy":"refund_unused","status":"source","source":"fixture"})).unwrap()];
    }
    for stock in &mut w.stockpiles {
        stock.currency_id = Some("COIN".into());
    }
    let mut initial = w.stockpiles[0].clone();
    initial.id = 12;
    initial.external_id = "initial-steel".into();
    initial.goods = Goods::Steel;
    initial.quantity = 2.;
    initial.inventory_origin = Some("opening".into());
    w.stockpiles.push(initial);
    w.accounts
        .push(production::account(100, "p", "personal_savings", 10000));
    w.rebuild_account_index().unwrap();
    w.market_data = market::market_world().market_data;
    w.market_data.territory.truncate(1);
    w.market_data.needs=Some(serde_json::from_value(json!({"schema_version":"weekly-needs-v1","profileRefs":[{"pop_id":"p","need_profile_id":"need","size":1,"wealth_band_id":"poor","literacy":0.5}],"profiles":[{"need_profile_id":"need","species_food_factor":1,"norms":[{"need_id":"equipment","goods_id":"steel","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":3,"unit":"t","allocation_rule":"none"}]}],"resolver":{"period":"week","period_days":7}})).unwrap());
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("steel".into(), json!(3));
    configure(&mut w);
    let opening = snapshot(&w);
    let basis = capture_income_opening(&w, &request()).unwrap();
    opening_arrivals(&mut w, "w");
    execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    execute_production_cycle(&mut w, &production::request()).unwrap();
    execute_market_phase(
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
    finish_supply(&mut w, "w");
    (w, basis, opening)
}
pub fn opening_arrivals(w: &mut WorldState, period: &str) {
    kingmaker_sim::shipment_arrivals::execute_shipment_arrivals(
        w,
        &kingmaker_core::transport::TransportRequest {
            expected_revision: w.campaign.revision,
            period: period.into(),
            stage: kingmaker_core::transport::TransportStage::Arrivals,
        },
    )
    .unwrap();
}
pub fn finish_supply(w: &mut WorldState, period: &str) {
    let r = kingmaker_core::transport::TransportRequest {
        expected_revision: w.campaign.revision,
        period: period.into(),
        stage: kingmaker_core::transport::TransportStage::LocalFoodReserves,
    };
    kingmaker_sim::local_food_reserves::execute_local_food_reserves(w, &r).unwrap();
    kingmaker_sim::physical_supply::execute_physical_supply(
        w,
        &kingmaker_core::transport::TransportRequest {
            stage: kingmaker_core::transport::TransportStage::Dispatch,
            ..r
        },
    )
    .unwrap();
    kingmaker_sim::raw_procurement::execute_raw_procurement(
        w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: w.campaign.revision,
            period: period.into(),
            policies: w.production_data.policies.clone(),
        },
    )
    .unwrap();
}
pub fn oracle(w: &WorldState, opening: Value) -> Value {
    oracle_action(w, opening, "producer_recognition")
}
pub fn oracle_action(w: &WorldState, opening: Value, action: &str) -> Value {
    let mut p = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/income-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut s = snapshot(w);
    s["registry"]["productionBeneficiaries"] = json!(w.mobility_data.production_beneficiaries);
    s["registry"]["factoryBeneficiaries"] = json!(w.mobility_data.factory_beneficiaries);
    s["registry"]["capitalistEntities"] = json!(w.mobility_data.capitalist_entities);
    s["registry"]["foundationParameters"] = json!(w.market_data.parameters);
    s["registry"]["maintenanceAgreements"] = json!(w.market_data.maintenance_agreements);
    p.stdin.take().unwrap().write_all(json!({"action":action,"snapshot":s,"opening":opening,"operations":w.production_data.committed_operations.iter().map(|op|&op.value).collect::<Vec<_>>(),"period":"w","policies":w.production_data.policies}).to_string().as_bytes()).unwrap();
    let output = p.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
