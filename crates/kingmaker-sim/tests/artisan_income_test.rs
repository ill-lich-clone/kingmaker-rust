#[path = "common/income.rs"]
mod common;
use common::{fixture_configure, oracle_action, request};
use kingmaker_core::*;
use kingmaker_sim::{artisan_income::plan_artisan_income, income_basis::IncomeOpeningBasis};
use serde_json::{json, Value};

fn fixture(shared: bool) -> (WorldState, IncomeOpeningBasis, Value) {
    fixture_custom(shared, |_| {})
}
fn fixture_custom(
    shared: bool,
    configure: impl FnOnce(&mut WorldState),
) -> (WorldState, IncomeOpeningBasis, Value) {
    fixture_configure(false, |w| {
        let factory = w.production_data.factories.remove(0);
        let mut artisan = serde_json::to_value(factory).unwrap();
        for field in [
            "capacity_source",
            "equipment_capacity_weekly",
            "equipment_source",
            "parameters",
            "staff_count",
            "tenure_days",
        ] {
            artisan.as_object_mut().unwrap().remove(field);
        }
        artisan["id"] = json!("a1");
        artisan["method_id"] = json!("artisan:steel");
        artisan["income_policy"] =
            json!({"source":"fixture","cash_reserve_minor":"0","payout_fraction":1});
        artisan["people_slots"] = json!(1);
        artisan["assignments"][0]
            .as_object_mut()
            .unwrap()
            .remove("contract_id");
        w.production_data
            .artisans
            .push(serde_json::from_value(artisan.clone()).unwrap());
        w.production_data.contracts.clear();
        let method = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
        method.method_id = "artisan:steel".into();
        method.kind = "artisan".into();
        method.staffing = vec!["artisans".into()];
        w.pops[0].pop_type_id = "artisans".into();
        w.pops[0].pop_type = "artisans".into();
        w.accounts[1].balance = Money(1000);
        if shared {
            artisan["id"] = json!("a2");
            artisan["assignments"][0]["employment_id"] = json!("e2");
            w.production_data
                .artisans
                .push(serde_json::from_value(artisan).unwrap());
            w.pops[0].size = 2;
            w.pops[0].workforce = 2.;
            w.pops[0].employed = 2.;
            w.stockpiles[0].quantity *= 2.;
            w.stockpiles[1].quantity *= 2.;
            w.market_data.needs.as_mut().unwrap().profiles[0].norms[0]
                .extra
                .insert("quantity_per_person_week".into(), json!(2));
            w.market_data.needs.as_mut().unwrap().profile_refs[0]
                .extra
                .insert("size".into(), json!(2));
        }
        configure(w);
        w.refresh_enterprise_views().unwrap();
    })
}
fn plan(w: &WorldState, basis: &IncomeOpeningBasis) -> Value {
    let before = serde_json::to_value(w).unwrap();
    let result = serde_json::to_value(plan_artisan_income(w, &request(), basis).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(w).unwrap(), before);
    result
}
#[test]
fn actual_single_artisan_sales_costs_and_tax_match_source() {
    let (mut w, basis, opening) = fixture(false);
    w.production_data.policies.taxes.insert("s".into(), 0.1);
    let actual = plan(&w, &basis);
    let mut expected = oracle_action(&w, opening, "artisan_plan");
    let ops: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(expected["operations"].clone()).unwrap();
    expected["operations"] = json!(ops);
    assert_eq!(actual["operations"], expected["operations"]);
    let expected_enterprises: Vec<kingmaker_core::production::ArtisanEnterprise> =
        serde_json::from_value(expected["artisans"].clone()).unwrap();
    assert_eq!(actual["enterprises"], json!(expected_enterprises));
    let history = &actual["enterprises"][0]["income_history"];
    assert_eq!(history["revenue_minor"], json!("300"));
    assert_eq!(history["recognized_input_cost_minor"], json!("150"));
    assert_eq!(history["opening_stock_revenue_minor"], json!("600"));
    assert_eq!(history["taxable_distribution_minor"], json!("150"));
    assert_eq!(actual["operations"][0]["tax_minor"], json!("15"));
}
#[test]
fn shared_owner_actual_output_and_sale_are_counted_once() {
    let (w, basis, _) = fixture(true);
    let actual = plan(&w, &basis);
    let histories: Vec<_> = actual["enterprises"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| &e["income_history"])
        .collect();
    assert_eq!(histories.len(), 2);
    assert_eq!(
        histories
            .iter()
            .map(|h| h["revenue_minor"].as_str().unwrap().parse::<i64>().unwrap())
            .sum::<i64>(),
        600
    );
    assert_eq!(
        histories
            .iter()
            .map(|h| h["taxable_distribution_minor"]
                .as_str()
                .unwrap()
                .parse::<i64>()
                .unwrap())
            .sum::<i64>(),
        300
    );
    assert!(histories.iter().all(|h| h["revenue_minor"] == json!("300")));
    assert!(histories
        .iter()
        .all(|h| h["recognized_input_cost_minor"] == json!("150")));
}
#[test]
fn losses_authored_reserve_and_payout_fraction_remain_effective() {
    let (mut w, basis, opening) = fixture(false);
    let e = &mut w.production_data.artisans[0];
    e.base.extra.insert("loss_carry_minor".into(), json!("50"));
    e.base
        .extra
        .insert("undistributed_profit_minor".into(), json!("40"));
    e.base.extra.get_mut("income_policy").unwrap()["payout_fraction"] = json!(0.5);
    let actual = plan(&w, &basis);
    assert_eq!(actual["enterprises"][0]["loss_carry_minor"], json!("0"));
    assert_eq!(
        actual["enterprises"][0]["undistributed_profit_minor"],
        json!("140")
    );
    assert_eq!(
        actual["enterprises"][0]["income_history"]["taxable_distribution_minor"],
        json!("50")
    );
    let expected: Vec<kingmaker_core::production::ArtisanEnterprise> =
        serde_json::from_value(oracle_action(&w, opening, "artisan_plan")["artisans"].clone())
            .unwrap();
    assert_eq!(actual["enterprises"], json!(expected));
    w.production_data.artisans[0]
        .base
        .extra
        .get_mut("income_policy")
        .unwrap()["cash_reserve_minor"] = json!("2000");
    assert!(plan(&w, &basis)["operations"]
        .as_array()
        .unwrap()
        .is_empty());
}
#[test]
fn unknown_input_quote_preserves_loss_and_blocks_payout_with_explicit_diagnostic() {
    let (mut w, basis, _) = fixture(false);
    w.production_data.artisans[0]
        .base
        .extra
        .insert("loss_carry_minor".into(), json!("50"));
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .remove("iron");
    let actual = plan(&w, &basis);
    assert!(actual["operations"].as_array().unwrap().is_empty());
    assert_eq!(actual["enterprises"][0]["loss_carry_minor"], json!("50"));
    assert!(actual["enterprises"][0]["income_history"]["net_income_minor"].is_null());
    assert!(actual["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["measurement"] == json!("нет измерения")));
}
#[test]
fn wrong_recipient_currency_fails_without_mutating_world() {
    let (mut w, basis, _) = fixture(false);
    w.pops[0].currency_id = Some("FOREIGN".into());
    let before = serde_json::to_value(&w).unwrap();
    assert!(plan_artisan_income(&w, &request(), &basis).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}

#[test]
fn real_independent_fte_income_splits_integer_remainder_and_tax() {
    let (w, basis, opening) = fixture_custom(false, |w| {
        let mut q = w.pops[0].clone();
        q.id = PopId(8);
        q.external_id = "q".into();
        w.pops.push(q);
        let e = &mut w.production_data.artisans[0];
        e.people_slots = Some(2.);
        e.base.assignments[0].time_fraction = 0.25;
        let mut second = e.base.assignments[0].clone();
        second.pop_id = "q".into();
        second.employment_id = Some("eq".into());
        second.time_fraction = 0.75;
        e.base.assignments.push(second);
        w.production_data.policies.taxes.insert("s".into(), 0.1);
    });
    let result = plan(&w, &basis);
    assert_eq!(result["operations"][0]["paid_gross_minor"], json!("37"));
    assert_eq!(result["operations"][1]["paid_gross_minor"], json!("113"));
    let expected: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(oracle_action(&w, opening, "artisan_plan")["operations"].clone())
            .unwrap();
    assert_eq!(result["operations"], json!(expected));
}

#[test]
fn actual_property_income_goes_to_owner_pop_and_paid_rent_reduces_principal_once() {
    let (w, basis, opening) = fixture_custom(false, |w| {
        let mut owner = w.pops[0].clone();
        owner.id = PopId(8);
        owner.external_id = "o".into();
        owner.workforce = 0.;
        owner.employed = 0.;
        w.pops.push(owner);
        w.production_data.artisans[0].base.assignments.clear();
        w.production_data.artisans[0].working_capital =
            Some(json!({"currency_id":"COIN","personal_funded_minor":"10"}));
        w.stockpiles[2].inventory_origin = Some("produced".into());
        w.market_data.sale_offers.push(serde_json::from_value(json!({"id":"property-offer","stock_id":"initial-steel","seller_id":"o","goods_id":"steel","seller_purpose":"operating_capital","status":"approved","source":"fixture","approval_ref":"fixture"})).unwrap());
    });
    let result = plan(&w, &basis);
    assert_eq!(result["operations"][0]["income_kind"], json!("property"));
    assert_eq!(result["operations"][0]["pop_id"], json!("o"));
    assert_eq!(result["operations"][0]["paid_gross_minor"], json!("600"));
    let expected: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(oracle_action(&w, opening, "artisan_plan")["operations"].clone())
            .unwrap();
    assert_eq!(result["operations"], json!(expected));
    let (w, basis, _) = fixture_custom(false, |w| {
        w.production_data.artisans[0].working_capital =
            Some(json!({"currency_id":"COIN","personal_funded_minor":"10"}));
        w.production_data.storage_facilities[0].owner_id = "landlord".into();
        w.production_data.storage_facilities[0].access_contracts=vec![serde_json::from_value(json!({"owner_id":"o","currency_id":"COIN","purpose":"operating_capital","price_minor_per_space":"5","cancellation_policy":"refund_unused","status":"source","source":"fixture"})).unwrap()];
    });
    let result = plan(&w, &basis);
    assert_eq!(
        result["enterprises"][0]["working_capital"]["personal_funded_minor"],
        json!("5")
    );
    assert_eq!(
        result["enterprises"][0]["income_history"]["recognized_input_cost_minor"],
        json!("155")
    );
}

#[test]
fn shared_owner_paid_rent_is_linked_to_each_actual_output_reservation() {
    let (w, basis, _) = fixture_custom(true, |w| {
        w.production_data.storage_facilities[0].owner_id = "landlord".into();
        w.production_data.storage_facilities[0].access_contracts=vec![serde_json::from_value(json!({"owner_id":"o","currency_id":"COIN","purpose":"operating_capital","price_minor_per_space":"5","cancellation_policy":"refund_unused","status":"source","source":"fixture"})).unwrap()];
    });
    let result = plan(&w, &basis);
    let enterprises = result["enterprises"].as_array().unwrap();
    assert!(enterprises
        .iter()
        .all(|e| e["income_history"]["recognized_input_cost_minor"] == json!("155")));
    assert!(enterprises
        .iter()
        .all(|e| e["income_history"]["taxable_distribution_minor"] == json!("145")));
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .all(|d| d["reason"] != json!("shared_artisan_debit_producer_unresolved")));
}

#[test]
fn pooled_inventory_contributions_survive_native_roundtrip_and_next_real_sale() {
    let (mut w, basis, _) = fixture_custom(true, |w| {
        w.market_data.needs.as_mut().unwrap().profiles[0].norms[0]
            .extra
            .insert("quantity_per_person_week".into(), json!(1.5));
    });
    let first = plan_artisan_income(&w, &request(), &basis).unwrap();
    assert_eq!(first.stock_contributions.len(), 2);
    // The actual market sells 1.5 from each of the opening and produced lots.
    assert!(first
        .stock_contributions
        .iter()
        .all(|c| (c.quantity - 0.25).abs() < 1e-9));
    w.income_data.stock_contributions = first.stock_contributions;
    let mut w: WorldState = serde_json::from_value(serde_json::to_value(w).unwrap()).unwrap();
    let mut r = request();
    r.period = "w2".into();
    let basis = kingmaker_sim::income_basis::capture_income_opening(&w, &r).unwrap();
    common::opening_arrivals(&mut w, "w2");
    kingmaker_sim::budget_cycle::execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w2".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    kingmaker_sim::production::execute_production_cycle(
        &mut w,
        &kingmaker_core::production::ProductionRequest {
            expected_revision: 20,
            period: "w2".into(),
            policies: Default::default(),
            profile: Default::default(),
            maintenance_receipts: vec![],
        },
    )
    .unwrap();
    kingmaker_sim::market_cycle::execute_market_phase(
        &mut w,
        &kingmaker_core::market::MarketRequest {
            expected_revision: 20,
            period: "w2".into(),
            stage: kingmaker_core::market::MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: None,
        },
    )
    .unwrap();
    common::finish_supply(&mut w, "w2");
    let second = plan_artisan_income(&w, &r, &basis).unwrap();
    assert!(second.stock_contributions.is_empty());
    let revenues: Vec<i64> = second
        .enterprises
        .iter()
        .map(|e| {
            e.base.extra["income_history"]["revenue_minor"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    // Each paid purchase rounds whole minor units; the final recipient gets its remainder.
    assert_eq!(revenues.iter().sum::<i64>(), 150);
    assert!(revenues.iter().all(|r| (74..=76).contains(r)));
}

#[test]
fn unknown_legacy_pool_is_not_allocated_by_capital_and_does_not_justify_payout() {
    let (w, basis, _) = fixture_custom(true, |w| {
        w.stockpiles[2].inventory_origin = Some("produced".into());
        w.stockpiles[2].facility_id = Some("wh".into());
    });
    let result = plan(&w, &basis);
    assert!(result["operations"].as_array().unwrap().is_empty());
    assert!(result["enterprises"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["income_history"]["net_income_minor"].is_null()));
    assert!(result["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["reason"] == json!("produced_sale_provenance_unresolved")));
}

#[test]
fn artisan_cost_uses_account_currency_quote_not_foreign_stock_purchase_currency() {
    let (w, basis, opening) = fixture_custom(false, |w| {
        w.stockpiles[0].currency_id = Some("FOREIGN".into());
        w.market_data
            .price_calibration
            .as_mut()
            .unwrap()
            .prices_by_currency
            .insert(
                "FOREIGN".into(),
                std::collections::BTreeMap::from([("iron".into(), json!(2))]),
            );
    });
    let result = plan(&w, &basis);
    assert_eq!(
        result["enterprises"][0]["income_history"]["recognized_input_cost_minor"],
        json!("150")
    );
    let expected: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(oracle_action(&w, opening, "artisan_plan")["operations"].clone())
            .unwrap();
    assert_eq!(result["operations"], json!(expected));
}

#[test]
fn working_capital_purchase_reserve_and_authored_minimum_protect_actual_cash() {
    let (mut w, basis, opening) = fixture(false);
    w.production_data.artisans[0].working_capital =
        Some(json!({"currency_id":"COIN","personal_funded_minor":"100"}));
    for (id, goods) in [(40, Goods::Iron), (41, Goods::Coal)] {
        let mut stock = w.stockpiles[0].clone();
        stock.id = id;
        stock.external_id = format!("vendor:{id}").into();
        stock.goods = goods;
        stock.owner_id = "vendor".into();
        stock.quantity = 10.;
        w.stockpiles.push(stock);
    }
    w.accounts
        .iter_mut()
        .find(|a| a.owner_id == ExternalId::from("o") && a.purpose == "operating_capital")
        .unwrap()
        .balance = Money(200);
    let capital = kingmaker_sim::income_capital::working_capital_requirements(&w, &[]).unwrap();
    assert_eq!(capital.groups[0].minimum_minor, Money(150));
    let result = plan(&w, &basis);
    assert_eq!(
        result["enterprises"][0]["income_history"]["taxable_distribution_minor"],
        json!("50")
    );
    assert_eq!(
        result["enterprises"][0]["working_capital"]["personal_funded_minor"],
        json!("100")
    );
    let expected: Vec<kingmaker_core::operations::EconomicOperation> = serde_json::from_value(
        oracle_action(&w, opening.clone(), "artisan_plan")["operations"].clone(),
    )
    .unwrap();
    assert_eq!(result["operations"], json!(expected));
    w.production_data.artisans[0]
        .base
        .extra
        .get_mut("income_policy")
        .unwrap()["cash_reserve_minor"] = json!("175");
    assert_eq!(
        plan(&w, &basis)["enterprises"][0]["income_history"]["taxable_distribution_minor"],
        json!("25")
    );
}

#[test]
fn absent_or_wrong_class_recipient_and_invalid_tax_cannot_publish_income() {
    for case in 0..3 {
        let (mut w, basis, _) = fixture(false);
        match case {
            0 => w.production_data.artisans[0].base.assignments[0].pop_id = "missing".into(),
            1 => w.pops[0].pop_type_id = "clerks".into(),
            _ => {
                w.production_data.policies.taxes.insert("s".into(), 1.5);
            }
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(plan_artisan_income(&w, &request(), &basis).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}

#[test]
fn no_recipient_keeps_profit_and_does_not_record_an_unpaid_distribution() {
    let (w, basis, opening) = fixture_custom(false, |w| {
        w.production_data.artisans[0].base.assignments.clear();
        w.stockpiles[2].inventory_origin = Some("produced".into());
    });
    let old = oracle_action(&w, opening, "artisan_plan");
    assert!(old["operations"].as_array().unwrap().is_empty());
    // Reproduced legacy bug: the old history says paid600 although no recipient or payment exists.
    assert_eq!(
        old["artisans"][0]["income_history"]["taxable_distribution_minor"],
        json!("600")
    );
    let result = plan(&w, &basis);
    assert!(result["operations"].as_array().unwrap().is_empty());
    assert_eq!(
        result["enterprises"][0]["income_history"]["taxable_distribution_minor"],
        json!("0")
    );
    assert_eq!(
        result["enterprises"][0]["undistributed_profit_minor"],
        json!("600")
    );
}
