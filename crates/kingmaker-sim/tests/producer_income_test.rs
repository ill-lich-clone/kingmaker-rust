#[path = "common/income.rs"]
mod common;
use common::{fixture, fixture_configure, oracle_action, request};
use kingmaker_core::*;
use kingmaker_sim::producer_income::plan_producer_income;
use serde_json::{json, Value};

fn rights(w: &mut WorldState) {
    for (id, name) in [(200, "aristo"), (201, "capital")] {
        let mut p = w.pops[0].clone();
        p.id = PopId(id);
        p.external_id = name.into();
        p.pop_type_id = if name == "aristo" {
            "aristocrats"
        } else {
            "capitalists"
        }
        .into();
        p.pop_type = p.pop_type_id.as_str().unwrap().into();
        p.workforce = 0.;
        p.employed = 0.;
        p.unemployed = 0.;
        p.account_id = None;
        p.account_status = "unbound".into();
        w.pops.push(p);
    }
    w.mobility_data.production_beneficiaries = vec![
        json!({"owner_id":"o","currency_id":"COIN","source":"authored ownership","beneficiaries":[{"owner_id":"aristo","purpose":"personal_savings","pop_id":"aristo","state_id":"s","numerator":"3","denominator":"5"},{"owner_id":"capital","purpose":"personal_savings","pop_id":"capital","state_id":"s","numerator":"1","denominator":"4"},{"owner_id":"treasury:s","purpose":"operating_capital","state_id":"s","numerator":"3","denominator":"20"}]}),
    ];
    w.production_data.policies.taxes.insert("s".into(), 0.1);
}
fn check(
    w: &WorldState,
    basis: &kingmaker_sim::income_basis::IncomeOpeningBasis,
    opening: Value,
) -> Value {
    let before = serde_json::to_value(w).unwrap();
    let actual = serde_json::to_value(plan_producer_income(w, &request(), basis).unwrap()).unwrap();
    let mut expected = oracle_action(w, opening, "producer_plan");
    let ops: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(expected["operations"].clone()).unwrap();
    expected["operations"] = json!(ops);
    assert_eq!(actual, expected);
    assert_eq!(serde_json::to_value(w).unwrap(), before);
    actual
}
#[test]
fn actual_profit_uses_authored_shares_tax_and_real_two_week_wage_reserve() {
    let (mut w, basis, opening) = fixture();
    rights(&mut w);
    let r = check(&w, &basis, opening);
    assert_eq!(r["history"][0]["net_income_minor"], json!("140"));
    assert_eq!(r["history"][0]["operating_reserve_minor"], json!("20"));
    assert_eq!(r["history"][0]["distributed_minor"], json!("140"));
    assert_eq!(
        r["operations"][0]["debits"][0]["amount_minor"],
        json!("140")
    );
    assert_eq!(
        r["operations"][0]["credits"][0]["amount_minor"],
        json!("76")
    );
}
#[test]
fn prior_loss_reduces_basis_and_unresolved_quotes_do_not_erase_it() {
    let (mut w, basis, opening) = fixture();
    rights(&mut w);
    w.income_data.producer_income =
        vec![json!({"owner_id":"o","currency_id":"COIN","loss_carry_minor":"100"})];
    assert_eq!(
        check(&w, &basis, opening)["history"][0]["distributed_minor"],
        json!("40")
    );
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .remove("iron");
    let r = plan_producer_income(&w, &request(), &basis).unwrap();
    let v = serde_json::to_value(r).unwrap();
    assert!(v["operations"].as_array().unwrap().is_empty());
    assert!(v["history"][0]["net_income_minor"].is_null());
    assert_eq!(v["history"][0]["loss_carry_minor"], json!("100"));
}
#[test]
fn no_rights_no_free_cash_and_loss_prevent_invented_dividends() {
    let (mut w, basis, opening) = fixture();
    assert!(check(&w, &basis, opening)["operations"]
        .as_array()
        .unwrap()
        .is_empty());
    rights(&mut w);
    w.accounts
        .iter_mut()
        .find(|a| a.owner_id == ExternalId::from("o") && a.purpose == "employer_operating_capital")
        .unwrap()
        .balance = Money(20);
    let r = serde_json::to_value(plan_producer_income(&w, &request(), &basis).unwrap()).unwrap();
    assert_eq!(r["history"][0]["distributed_minor"], json!("0"));
    w.income_data.producer_income =
        vec![json!({"owner_id":"o","currency_id":"COIN","loss_carry_minor":"1000"})];
    assert_eq!(
        serde_json::to_value(plan_producer_income(&w, &request(), &basis).unwrap()).unwrap()
            ["history"][0]["loss_carry_minor"],
        json!("860")
    );
}
#[test]
fn authored_twenty_five_day_input_buffer_debt_and_maintenance_reserves_are_preserved() {
    let (mut w, basis, opening) = fixture();
    rights(&mut w);
    w.market_data.parameters.as_mut().unwrap().extra.insert(
        "operating_reserve_days".into(),
        json!({"source":"authored","value":25}),
    );
    w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"debt","payer_id":"o","payee_id":"p","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"100"})).unwrap());
    w.market_data.maintenance_agreements.push(serde_json::from_value(json!({"maintenance_id":"maintenance","payer_id":"o","pop_id":"p","currency_id":"COIN","weekly_amount_minor":"7"})).unwrap());
    let r = check(&w, &basis, opening);
    assert_eq!(r["history"][0]["operating_reserve_minor"], json!("670")); //20 wages +536 quoted inputs +100 debt +14 maintenance.
}
#[test]
fn malformed_shares_and_missing_project_investor_fail_without_publication() {
    for case in 0..3 {
        let (mut w, basis, _) = fixture();
        rights(&mut w);
        if case == 0 {
            w.mobility_data.production_beneficiaries[0]["beneficiaries"][0]["denominator"] =
                json!("0");
        } else if case == 1 {
            w.mobility_data.production_beneficiaries[0]["beneficiaries"][0]["numerator"] =
                json!("2");
        } else {
            w.mobility_data.production_beneficiaries.clear();
            w.production_data.factories[0].base.extra.insert("shares".into(),json!([{"investor_id":"missing","owner_id":"capital","numerator":"1","denominator":"1"}]));
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(plan_producer_income(&w, &request(), &basis).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn actual_completed_project_shares_pay_the_verified_capitalist() {
    let (mut w, basis, opening) = fixture();
    rights(&mut w);
    w.mobility_data.production_beneficiaries.clear();
    w.mobility_data.capitalist_entities = vec![
        json!({"id":"investor","owner_id":"capital","pop_id":"capital","account_purpose":"personal_savings"}),
    ];
    w.production_data.factories[0].base.extra.insert(
        "shares".into(),
        json!([{"investor_id":"investor","owner_id":"capital","numerator":"1","denominator":"1"}]),
    );
    let r = check(&w, &basis, opening);
    assert_eq!(
        r["operations"][0]["credits"][0]["owner_id"],
        json!("capital")
    );
    assert_eq!(
        r["operations"][0]["credits"][0]["amount_minor"],
        json!("126")
    );
}
#[test]
fn factory_override_does_not_award_factory_profit_to_rgo_owners() {
    let (mut w, basis, opening) = fixture_configure(false, |w| {
        w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"rgo:grain","kind":"rgo","status":"ready","output_goods_id":"grain","output_unit":"t","weekly_productivity":1,"staffing":["farmers"],"inputs":[]})).unwrap());
        w.production_data.catalog.as_mut().unwrap().goods.push(
            serde_json::from_value(json!({"goods_id":"grain","kind":"raw_resource","unit":"t"}))
                .unwrap(),
        );
        w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"raw","owner_id":"o","province_id":1,"method_id":"rgo:grain","resource_id":"grain","resource_status":"approved","approval_ref":"source","land_access":true,"assignments":[]})).unwrap());
    });
    rights(&mut w);
    w.mobility_data.factory_beneficiaries = vec![
        json!({"owner_id":"o","currency_id":"COIN","source":"retained factory rights","beneficiaries":[{"owner_id":"capital","purpose":"personal_savings","pop_id":"capital","state_id":"s","numerator":"1","denominator":"1"}]}),
    ];
    let r = check(&w, &basis, opening);
    assert_eq!(
        r["operations"][0]["credits"][0]["owner_id"],
        json!("capital")
    );
    assert_eq!(r["history"][0]["distributed_minor"], json!("140"));
}
#[test]
fn actual_rgo_and_factory_profit_share_one_cash_pool_with_sector_rights() {
    let (mut w, basis, opening) = fixture_configure(false, |w| {
        w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"rgo:grain","kind":"rgo","status":"ready","output_goods_id":"grain","output_unit":"t","weekly_productivity":1,"staffing":["farmers"],"inputs":[]})).unwrap());
        w.production_data.catalog.as_mut().unwrap().goods.push(
            serde_json::from_value(json!({"goods_id":"grain","kind":"raw_resource","unit":"t"}))
                .unwrap(),
        );
        let mut p = w.pops[0].clone();
        p.id = PopId(300);
        p.external_id = "farmer".into();
        p.pop_type_id = "farmers".into();
        p.pop_type = "farmers".into();
        p.extra.insert("species_labor_coefficient".into(), json!(1));
        w.pops.push(p);
        w.production_data.contracts.push(serde_json::from_value(json!({"contract_id":"c:raw","employer_entity_id":"o","currency_id":"COIN","weekly_wage_rate":{"numerator":"1","denominator":"1"},"guaranteed_idle_weekly_rate":{"numerator":"0","denominator":"1"}})).unwrap());
        w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"raw","owner_id":"o","province_id":1,"method_id":"rgo:grain","facility_id":"wh","resource_id":"grain","resource_status":"approved","approval_ref":"source","land_access":true,"assignments":[{"pop_id":"farmer","workers_count":1,"time_fraction":1,"employment_id":"e:raw","contract_id":"c:raw"}]})).unwrap());
        w.production_data.storage_facilities[0].goods.insert(
            "grain".into(),
            serde_json::from_value(json!({"unit":"t","space_per_unit":1})).unwrap(),
        );
        w.market_data.needs.as_mut().unwrap().profiles[0].norms.push(serde_json::from_value(json!({"need_id":"test-grain","goods_id":"grain","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":1,"unit":"t","allocation_rule":"none"})).unwrap());
    });
    rights(&mut w);
    w.mobility_data.factory_beneficiaries = vec![
        json!({"owner_id":"o","currency_id":"COIN","source":"retained factory rights","beneficiaries":[{"owner_id":"capital","purpose":"personal_savings","pop_id":"capital","state_id":"s","numerator":"1","denominator":"1"}]}),
    ];
    let r = check(&w, &basis, opening);
    assert_eq!(r["history"][0]["net_income_minor"], json!("239"));
    assert_eq!(r["history"][0]["operating_reserve_minor"], json!("22"));
    assert_eq!(r["history"][0]["distributed_minor"], json!("239"));
    assert_eq!(r["operations"].as_array().unwrap().len(), 1);
    assert_eq!(
        r["operations"][0]["debits"][0]["amount_minor"],
        json!("239")
    );
    assert_eq!(
        r["operations"][0]["credits"][0]["amount_minor"],
        json!("53")
    );
}
#[test]
fn zero_or_partial_explicit_tax_override_preserves_other_authored_policies() {
    let (mut w, basis, _) = fixture();
    rights(&mut w);
    w.production_data
        .policies
        .taxes
        .insert("unrelated".into(), 0.35);
    let mut r = request();
    r.policies.taxes.insert("s".into(), 0.);
    let plan = serde_json::to_value(plan_producer_income(&w, &r, &basis).unwrap()).unwrap();
    assert_eq!(
        plan["operations"][0]["credits"][0]["amount_minor"],
        json!("84")
    );
    assert_eq!(w.production_data.policies.taxes["unrelated"], 0.35);
    let mut r = request();
    r.policies.wages.insert("o".into(), 0.5);
    assert_eq!(
        serde_json::to_value(plan_producer_income(&w, &r, &basis).unwrap()).unwrap()["history"][0]
            ["operating_reserve_minor"],
        json!("10")
    );
}
#[test]
fn actual_recognized_loss_is_carried_without_distribution_or_repricing_old_sales() {
    let (mut w, basis, opening) = fixture();
    rights(&mut w);
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("iron".into(), json!(100));
    let r = check(&w, &basis, opening);
    assert_eq!(r["history"][0]["revenue_minor"], json!("300"));
    assert_eq!(r["history"][0]["recognized_cost_minor"], json!("11050"));
    assert_eq!(r["history"][0]["loss_carry_minor"], json!("10750"));
    assert!(r["operations"].as_array().unwrap().is_empty());
}
#[test]
fn unknown_buffer_version_and_wrong_beneficiary_currency_reject_before_payment() {
    for case in 0..2 {
        let (mut w, basis, _) = fixture();
        rights(&mut w);
        if case == 0 {
            w.market_data.parameters.as_mut().unwrap().extra.insert(
                "operating_reserve_days".into(),
                json!({"source":"authored","value":26}),
            );
        } else {
            w.pops
                .iter_mut()
                .find(|p| p.external_id == ExternalId::from("aristo"))
                .unwrap()
                .currency_id = Some("OTHER".into());
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(plan_producer_income(&w, &request(), &basis).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
