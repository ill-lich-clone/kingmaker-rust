#[path = "common/budget.rs"]
mod budget;
#[path = "common/market.rs"]
mod market;
use kingmaker_core::{production::*, *};
use kingmaker_sim::{labor_money::*, labor_offers::JobOffer, labor_roster::build_labor_roster};
use serde_json::json;
use serde_json::Value;
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn oracle(input: Value) -> Value {
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/labor-oracle.cjs"
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
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn own_basket_and_monthly_extraction_rate_match_actual_js() {
    let mut w = basket_world();
    let p = &w.pops[0];
    let pop = json!({"pop_id":p.external_id,"province_id":1,"pop_type_id":p.pop_type_id,"size":p.size,"workforce":p.workforce,"currency_id":p.currency_id});
    let source = oracle(
        json!({"action":"basic_basket","snapshot":{"registry":{"needs":w.market_data.needs,"priceCalibration":w.market_data.price_calibration}},"pop":pop}),
    );
    assert_eq!(
        source,
        json!(basic_basket_price(&w, p).unwrap().0.to_string())
    );
    let entity = json!({"id":"asset","owner_id":"owner","province_id":1,"method_id":"rgo:iron","resource_id":"iron","resource_status":"approved","land_access":true,"assignments":[]});
    w.production_data
        .resource_operations
        .push(serde_json::from_value(entity.clone()).unwrap());
    let mut o = offer("rgo");
    o.method_id = Some("rgo:iron".into());
    let native =
        own_labor_contract(&w, &w.pops[0], &o, 100, &ProductionPolicies::default()).unwrap();
    let source = oracle(
        json!({"action":"rgo_contract","options":{"pop":pop,"entity":entity,"workers":100,"weeklyBasketMinor":"1600","taxRate":0}}),
    );
    assert_eq!(
        native.weekly_wage_rate.unwrap().numerator,
        source["weekly_wage_rate"]["numerator"]
    );
    assert_eq!(native.extra["organization"], source["organization"]);
    assert_eq!(native.extra["provenance"], source["provenance"]);
}
fn basket_world() -> WorldState {
    let mut w = market::market_world();
    w.market_data.needs.as_mut().unwrap().profiles[0].norms.push(serde_json::from_value(json!({"need_id":"clothing","goods_id":"clothing","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":0.002,"unit":"t","allocation_rule":"none"})).unwrap());
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("clothing".into(), json!(3));
    w
}
fn offer(kind: &str) -> JobOffer {
    JobOffer {
        id: "offer".into(),
        kind: kind.into(),
        entity_id: "asset".into(),
        province_id: ProvinceId(1),
        owner_id: "owner".into(),
        method_id: None,
        job: None,
        sector: None,
        capacity: 40000.,
        places: 40000,
        staffing: vec!["farmers".into()],
        purpose: "employer_operating_capital".into(),
    }
}
#[test]
fn own_basic_basket_uses_actual_population_and_exact_current_prices() {
    let mut w = basket_world();
    assert_eq!(basic_basket_price(&w, &w.pops[0]).unwrap(), Money(1600));
    w.pops[0].size = 500;
    assert_eq!(basic_basket_price(&w, &w.pops[0]).unwrap(), Money(800));
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .remove("clothing");
    assert!(basic_basket_price(&w, &w.pops[0]).is_err());
}
#[test]
fn source_own_contract_rates_preserve_monthly_rgo_rounding_and_role_multipliers() {
    let w = basket_world();
    let mut o = offer("rgo");
    o.method_id = Some("rgo:grain".into());
    let mut rw = w.clone();
    rw.production_data.resource_operations.push(serde_json::from_value(json!({"id":"asset","owner_id":"owner","province_id":1,"method_id":"rgo:grain","resource_id":"grain","resource_status":"approved","land_access":true,"assignments":[]})).unwrap());
    let c = own_labor_contract(&rw, &rw.pops[0], &o, 100, &ProductionPolicies::default()).unwrap();
    assert_eq!(c.weekly_wage_rate.as_ref().unwrap().numerator, "77");
    assert_eq!(c.weekly_wage_rate.as_ref().unwrap().denominator, "30");
    let o = offer("factory");
    let c = own_labor_contract(&w, &w.pops[0], &o, 100, &ProductionPolicies::default()).unwrap();
    assert_eq!(c.weekly_wage_rate.as_ref().unwrap().numerator, "184000");
    assert_eq!(c.weekly_wage_rate.as_ref().unwrap().denominator, "60000");
    for (role, pct) in [("teacher", 105), ("soldier", 120), ("officer", 135)] {
        let mut o = offer("service");
        o.sector = Some(
            if role == "teacher" {
                "education"
            } else {
                "army"
            }
            .into(),
        );
        o.job = Some(json!({"job_id":"job","role_id":role,"capacity_pool_id":"pool"}));
        let c =
            own_labor_contract(&w, &w.pops[0], &o, 100, &ProductionPolicies::default()).unwrap();
        assert_eq!(
            c.weekly_wage_rate.unwrap().numerator,
            (1600 * pct).to_string()
        );
    }
}
#[test]
fn available_cash_reserves_existing_wages_and_old_debt_without_spending_forecast() {
    let mut w = budget::world(4000);
    w.labor_data.profiles.push(json!({"state_id":"s"}));
    w.accounts.push(budget::account(
        11,
        "s",
        "X",
        "state_service:education",
        1000,
    ));
    w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"old","payer_id":"s","payee_id":"teacher","currency_id":"X","purpose":"state_service:education","amount_minor":"500"})).unwrap());
    w.rebuild_account_index().unwrap();
    let r = build_labor_roster(&w).unwrap();
    let cash = available_labor_cash(&w, &w.production_data.policies, &r).unwrap();
    assert_eq!(
        cash.amount(&"treasury:s".into(), &"X".into(), "operating_capital"),
        Money(500)
    );
    assert_eq!(
        cash.amount(&"s".into(), &"X".into(), "state_service:education"),
        Money::ZERO
    );
    assert_eq!(w.accounts[0].balance, Money(4000));
}
