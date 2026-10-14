#[path = "common/production.rs"]
mod production;
use kingmaker_core::*;
use kingmaker_sim::{
    labor_forecast::HiringForecast, labor_money::available_labor_cash,
    labor_roster::build_labor_roster,
};
use serde_json::json;
#[path = "common/labor.rs"]
mod common;
#[test]
fn incremental_raw_and_storage_reservations_match_actual_js() {
    let mut w = fixture();
    let mut second = w.production_data.factories[0].clone();
    second.base.id = "second".into();
    w.production_data.factories.push(second);
    let source = common::oracle(
        json!({"action":"forecast","snapshot":common::snapshot(&w),"steps":[{"entity_id":"f","pop_id":"p","requested":10,"accept":true},{"entity_id":"second","pop_id":"p","requested":10}]}),
    );
    let mut f = forecast(&w);
    let n = f
        .limit(&"f".into(), &w.pops[0], 10, |_, _| Ok(true))
        .unwrap();
    f.accept(&"f".into(), &w.pops[0], n).unwrap();
    let second = f
        .limit(&"second".into(), &w.pops[0], 10, |_, _| Ok(true))
        .unwrap();
    assert_eq!(source, json!([n, second]));
}
fn fixture() -> WorldState {
    let mut w = production::world();
    w.production_data.factories[0].base.assignments.clear();
    w.production_data.factories[0].staff_count = Some(0.);
    w.pops[0].size = 10;
    w.pops[0].workforce = 10.;
    w.pops[0].employed = 0.;
    w.stockpiles[0].quantity = 5.5;
    w.stockpiles[1].quantity = 2.;
    w
}
fn forecast(w: &WorldState) -> HiringForecast<'_> {
    let r = build_labor_roster(w).unwrap();
    let cash = available_labor_cash(w, &w.production_data.policies, &r).unwrap();
    HiringForecast::new(w, cash).unwrap()
}
#[test]
fn shared_inputs_and_space_are_reserved_once_without_real_production() {
    let mut w = fixture();
    let mut second = w.production_data.factories[0].clone();
    second.base.id = "second".into();
    w.production_data.factories.push(second);
    let before = serde_json::to_value(&w).unwrap();
    let mut f = forecast(&w);
    assert_eq!(
        f.limit(&"f".into(), &w.pops[0], 10, |_, _| Ok(true))
            .unwrap(),
        5
    );
    f.accept(&"f".into(), &w.pops[0], 5).unwrap();
    assert_eq!(
        f.limit(&"second".into(), &w.pops[0], 10, |_, _| Ok(true))
            .unwrap(),
        0
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn preview_restores_inputs_assets_and_money_even_after_error() {
    let w = fixture();
    let mut f = forecast(&w);
    let pool = AccountAddress {
        owner_id: "o".into(),
        currency_id: "COIN".into(),
        purpose: "employer_operating_capital".into(),
    };
    let result = f.preview(|f| {
        f.accept(&"f".into(), &w.pops[0], 4)?;
        assert!(f.reserve_wages(&pool, &pool, Money(10))?);
        Err::<(), _>(kingmaker_sim::operation_session::OperationError(
            "forced".into(),
        ))
    });
    assert!(result.is_err());
    assert_eq!(
        f.cash
            .amount(&"o".into(), &"COIN".into(), "employer_operating_capital"),
        Money(20)
    );
    assert_eq!(
        f.limit(&"f".into(), &w.pops[0], 10, |_, _| Ok(true))
            .unwrap(),
        5
    );
}
#[test]
fn current_output_and_warehouse_limit_incremental_hires() {
    let mut w = fixture();
    w.production_data.factories[0].base.assignments = production::world().production_data.factories
        [0]
    .base
    .assignments
    .clone();
    w.production_data.factories[0].staff_count = Some(1.);
    w.production_data.storage_facilities[0].capacity = Some(3.);
    let mut f = forecast(&w);
    assert_eq!(
        f.limit(&"f".into(), &w.pops[0], 10, |_, _| Ok(true))
            .unwrap(),
        2
    );
    w.production_data.storage_facilities[0].capacity = Some(0.);
    assert_eq!(
        forecast(&w)
            .limit(&"f".into(), &w.pops[0], 10, |_, _| Ok(true))
            .unwrap(),
        0
    );
}
#[test]
fn leased_space_and_wages_share_finite_money() {
    let mut w = fixture();
    let wh = &mut w.production_data.storage_facilities[0];
    wh.owner_id = "landlord".into();
    wh.access_contracts=serde_json::from_value(json!([{"owner_id":"o","status":"source","source":"fixture","price_minor_per_space":"4","currency_id":"COIN","purpose":"employer_operating_capital"}])).unwrap();
    let mut f = forecast(&w);
    assert_eq!(
        f.limit(&"f".into(), &w.pops[0], 10, |d, n| Ok(d
            .rent
            .as_ref()
            .unwrap()
            .1
             .0
            + n as i64 * 2
            <= 20))
            .unwrap(),
        3
    );
    f.accept(&"f".into(), &w.pops[0], 3).unwrap();
    assert_eq!(
        f.cash
            .amount(&"o".into(), &"COIN".into(), "employer_operating_capital"),
        Money(8)
    );
}
#[test]
fn support_requires_direct_workers_and_authored_ratio() {
    let mut w = fixture();
    w.pops[0].pop_type_id = "clerks".into();
    w.production_data.factories[0]
        .parameters
        .as_mut()
        .unwrap()
        .clerk_ratio = Some(0.2);
    w.production_data.factories[0]
        .parameters
        .as_mut()
        .unwrap()
        .clerk_bmax = Some(0.1);
    assert_eq!(
        forecast(&w)
            .limit(&"f".into(), &w.pops[0], 10, |_, _| Ok(true))
            .unwrap(),
        0
    );
}
