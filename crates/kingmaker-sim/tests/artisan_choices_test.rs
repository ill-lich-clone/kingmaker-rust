#[path = "common/income.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::artisan_choices::{
    build_artisan_quotes, choose_artisan_method, plan_artisan_choices,
};
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn fixture() -> WorldState {
    fixture_custom(|_| {})
}
fn fixture_custom(configure: impl FnOnce(&mut WorldState)) -> WorldState {
    fixture_with_basis(configure).0
}
fn fixture_with_basis(
    configure: impl FnOnce(&mut WorldState),
) -> (WorldState, kingmaker_sim::income_basis::IncomeOpeningBasis) {
    let (w, basis, _) = common::fixture_configure(false, |w| {
        w.production_data.factories.clear();
        w.production_data.contracts.clear();
        w.pops[0].pop_type_id = "artisans".into();
        w.pops[0].pop_type = "artisans".into();
        w.accounts[1].balance = Money(1000);
        let current = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
        current.method_id = "artisan:steel".into();
        current.kind = "artisan".into();
        current.staffing = vec!["artisans".into()];
        w.production_data.artisans.push(serde_json::from_value(json!({"id":"a","owner_id":"o","province_id":1,"facility_id":"wh","method_id":"artisan:steel","status":"operating","people_slots":1,"assignments":[{"employment_id":"ea","pop_id":"p","workers_count":1,"time_fraction":1}],"income_policy":{"source":"fixture","cash_reserve_minor":"0","payout_fraction":1},"automatic_choice":true,"retooling":{"source":"fixture","provider_id":"smith","currency_id":"COIN","purpose":"operating_capital","duration_days":10,"cost_minor":"20"}})).unwrap());
        w.stockpiles[0].quantity = 2.2;
        w.production_data.catalog.as_mut().unwrap().goods.push(
            serde_json::from_value(json!({"goods_id":"tools","kind":"processed_good","unit":"t"}))
                .unwrap(),
        );
        w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"artisan:tools","kind":"artisan","status":"ready","output_goods_id":"tools","output_unit":"t","weekly_productivity":1,"staffing":["artisans"],"technology":[],"inputs":[{"goods_id":"iron","amount":0.5}]})).unwrap());
        w.production_data.storage_facilities[0].goods.insert(
            "tools".into(),
            serde_json::from_value(json!({"unit":"t","space_per_unit":1})).unwrap(),
        );
        w.market_data
            .price_calibration
            .as_mut()
            .unwrap()
            .prices_by_currency
            .get_mut("COIN")
            .unwrap()
            .insert("tools".into(), json!(10));
        w.market_data.needs.as_mut().unwrap().profiles[0].norms.push(serde_json::from_value(json!({"need_id":"tools","goods_id":"tools","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":1,"unit":"t","allocation_rule":"none"})).unwrap());
        configure(w);
        w.refresh_enterprise_views().unwrap();
    });
    (w, basis)
}

#[test]
fn closed_income_pays_retooling_then_loaded_opening_completes_without_repaying() {
    use kingmaker_sim::income_cycle::{execute_artisan_opening, execute_income_closing};
    let (mut w, basis) = fixture_with_basis(|_| {});
    let request = common::request();
    let closing = execute_income_closing(&mut w, &request, &basis).unwrap();
    let provider_cash = |w: &WorldState| {
        w.accounts
            .iter()
            .filter(|a| a.owner_id == ExternalId::from("smith"))
            .map(|a| a.balance.0)
            .sum::<i64>()
    };
    assert_eq!(provider_cash(&w), 20);
    assert_eq!(
        w.production_data.artisans[0].base.method_id,
        ExternalId::from("artisan:steel")
    );
    assert_eq!(
        w.production_data.artisans[0]
            .pending_retooling
            .as_ref()
            .unwrap()["ready_date"],
        json!({"year":306,"month":1,"day":18})
    );
    let mut loaded: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    loaded.rebuild_account_index().unwrap();
    assert_eq!(
        execute_income_closing(&mut loaded, &request, &basis).unwrap(),
        closing
    );
    loaded.campaign.day = 8;
    let mut next = request.clone();
    next.period = "w2".into();
    execute_artisan_opening(&mut loaded, &next).unwrap();
    assert_eq!(provider_cash(&loaded), 20);
    assert_eq!(
        loaded.production_data.artisans[0].base.method_id,
        ExternalId::from("artisan:steel")
    );
    loaded.campaign.day = 18;
    next.period = "w3".into();
    let completed = execute_artisan_opening(&mut loaded, &next).unwrap();
    assert_eq!(provider_cash(&loaded), 20);
    assert!(loaded.production_data.artisans[0]
        .pending_retooling
        .is_none());
    assert_eq!(
        loaded.production_data.artisans[0].base.method_id,
        ExternalId::from("artisan:tools")
    );
    assert_eq!(
        execute_artisan_opening(&mut loaded, &next).unwrap(),
        completed
    );
    assert_eq!(provider_cash(&loaded), 20);
}
fn oracle(
    w: &WorldState,
    action: &str,
    quotes: &[Value],
    opening: bool,
    budget: Option<Money>,
) -> Value {
    let mut process = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/income-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let receipt = w.market_data.phase_receipts.last().unwrap();
    process.stdin.take().unwrap().write_all(json!({"action":action,"snapshot":common::snapshot(w),"period":"w","operations":receipt.operations,"orders":receipt.orders,"quotes":quotes,"opening_phase":opening,"enterprise_id":"a","capital_budget_minor":budget}).to_string().as_bytes()).unwrap();
    let output = process.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn quotes_use_actual_unfilled_orders_and_permitted_inputs_and_match_source() {
    let w = fixture();
    let before = serde_json::to_value(&w).unwrap();
    let actual = build_artisan_quotes(&w, &common::request()).unwrap();
    let mut expected = oracle(&w, "artisan_quotes", &[], false, None);
    for q in expected.as_array_mut().unwrap() {
        for key in ["price", "demand", "available"] {
            if let Some(n) = q.get(key).and_then(Value::as_f64) {
                q[key] = json!(n);
            }
        }
    }
    assert_eq!(json!(actual), expected);
    assert_eq!(
        actual
            .iter()
            .find(|q| q["goods_id"] == json!("tools"))
            .unwrap()["demand"],
        json!(1.)
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn closing_choice_pays_real_provider_and_starts_from_next_week_date() {
    let w = fixture();
    let before = serde_json::to_value(&w).unwrap();
    let r = common::request();
    let actual = plan_artisan_choices(&w, &r, false).unwrap();
    let quotes = build_artisan_quotes(&w, &r).unwrap();
    let mut expected = oracle(&w, "artisan_choices", &quotes, false, None);
    let ops: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(expected["operations"].clone()).unwrap();
    expected["operations"] = json!(ops);
    let enterprises: Vec<kingmaker_core::production::ArtisanEnterprise> =
        serde_json::from_value(expected["enterprises"].clone()).unwrap();
    expected["enterprises"] = json!(enterprises);
    let actual = serde_json::to_value(actual).unwrap();
    assert_eq!(actual["operations"], expected["operations"]);
    assert_eq!(actual["enterprises"], expected["enterprises"]);
    assert_eq!(
        actual["operations"][0]["debits"][0]["amount_minor"],
        json!("20")
    );
    assert_eq!(
        actual["enterprises"][0]["pending_retooling"]["ready_date"],
        json!({"year":306,"month":1,"day":18})
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn disabled_pending_technology_demand_or_cash_prevent_unfunded_switches() {
    for case in 0..5 {
        let mut w = fixture();
        match case {
            0 => {
                w.production_data.artisans[0]
                    .base
                    .extra
                    .insert("automatic_choice".into(), json!(false));
            }
            1 => {
                w.production_data.artisans[0].pending_retooling =
                    Some(json!({"method_id":"artisan:tools","remaining_days":10}))
            }
            2 => {
                w.production_data.catalog.as_mut().unwrap().methods[1].technology =
                    vec!["unknown-tech".into()]
            }
            3 => {
                w.production_data.catalog.as_mut().unwrap().methods[1].weekly_productivity =
                    Some(0.);
            }
            _ => {
                w.accounts
                    .iter_mut()
                    .find(|a| {
                        a.owner_id == ExternalId::from("o") && a.purpose == "operating_capital"
                    })
                    .unwrap()
                    .balance = Money(0);
            }
        };
        assert!(plan_artisan_choices(&w, &common::request(), false)
            .unwrap()
            .operations
            .is_empty());
    }
}
#[test]
fn pending_ready_date_overrides_countdown_and_completion_never_charges_again() {
    let mut w = fixture();
    w.production_data.artisans[0].pending_retooling = Some(
        json!({"event_id":"paid","method_id":"artisan:tools","remaining_days":1,"ready_date":{"year":306,"month":1,"day":18}}),
    );
    w.campaign.day = 8;
    let result = plan_artisan_choices(&w, &common::request(), true).unwrap();
    assert!(result.operations.is_empty());
    assert_eq!(
        result.enterprises[0].base.method_id,
        ExternalId::from("artisan:steel")
    );
    w.production_data.artisans = result.enterprises;
    w.campaign.day = 18;
    let mut w: WorldState = serde_json::from_value(serde_json::to_value(w).unwrap()).unwrap();
    let result = plan_artisan_choices(&w, &common::request(), true).unwrap();
    assert_eq!(
        result.enterprises[0].base.method_id,
        ExternalId::from("artisan:tools")
    );
    assert!(result.enterprises[0].pending_retooling.is_none());
    assert!(result
        .operations
        .iter()
        .all(|op| op.debits.is_empty() && op.credits.is_empty()));
    w.production_data.artisans = result.enterprises;
    assert!(plan_artisan_choices(&w, &common::request(), true)
        .unwrap()
        .operations
        .is_empty());
}
#[test]
fn exact_method_choice_binary_search_and_tie_sort_match_real_source() {
    let mut w = fixture();
    w.stockpiles
        .iter_mut()
        .filter(|s| s.goods == Goods::Iron)
        .for_each(|s| s.quantity = 0.);
    for id in ["artisan:tools-z", "artisan:tools-a"] {
        let mut method = w.production_data.catalog.as_ref().unwrap().methods[1].clone();
        method.method_id = id.into();
        w.production_data
            .catalog
            .as_mut()
            .unwrap()
            .methods
            .push(method);
    }
    let quotes = vec![
        json!({"province_id":1,"currency_id":"COIN","goods_id":"tools","price":10,"demand":10}),
        json!({"province_id":1,"currency_id":"COIN","goods_id":"iron","price":1,"available":10}),
    ];
    let actual = serde_json::to_value(
        choose_artisan_method(&w, &"a".into(), &quotes, Some(Money(25))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        actual,
        oracle(&w, "artisan_choose", &quotes, false, Some(Money(25)))
    );
}

#[test]
fn shared_owner_choice_uses_cash_after_previous_real_provider_payment() {
    let mut w = fixture_custom(|w| {
        let mut second = w.production_data.artisans[0].clone();
        second.base.id = "b".into();
        second.base.assignments[0].employment_id = Some("eb".into());
        w.production_data.artisans.push(second);
        w.pops[0].size = 2;
        w.pops[0].workforce = 2.;
        w.pops[0].employed = 2.;
        w.stockpiles[0].quantity *= 2.;
        w.stockpiles[1].quantity *= 2.;
    });
    w.accounts
        .iter_mut()
        .find(|a| a.owner_id == ExternalId::from("o") && a.purpose == "operating_capital")
        .unwrap()
        .balance = Money(20);
    let result = plan_artisan_choices(&w, &common::request(), false).unwrap();
    assert_eq!(result.operations.len(), 1);
    assert!(result.enterprises[0].pending_retooling.is_some());
    assert!(result.enterprises[1].pending_retooling.is_none());
}

#[test]
fn protected_food_stock_absent_raw_unprofitable_or_unrequested_output_do_not_trigger_choice() {
    for case in 0..4 {
        let mut w = fixture();
        match case {
            0 => w
                .stockpiles
                .iter_mut()
                .filter(|s| s.goods == Goods::Iron)
                .for_each(|s| s.purpose = "food_reserve".into()),
            1 => w
                .stockpiles
                .iter_mut()
                .filter(|s| s.goods == Goods::Iron)
                .for_each(|s| s.quantity = 0.),
            2 => {
                w.market_data
                    .price_calibration
                    .as_mut()
                    .unwrap()
                    .prices_by_currency
                    .get_mut("COIN")
                    .unwrap()
                    .insert("tools".into(), json!(0.4));
            }
            _ => {
                w.production_data.catalog.as_mut().unwrap().methods[1].output_goods_id =
                    "clothing".into()
            }
        }
        assert!(plan_artisan_choices(&w, &common::request(), false)
            .unwrap()
            .operations
            .is_empty());
    }
}

#[test]
fn invalid_provider_currency_or_terms_never_move_money() {
    for case in 0..4 {
        let mut w = fixture();
        let terms = w.production_data.artisans[0]
            .base
            .extra
            .get_mut("retooling")
            .unwrap();
        match case {
            0 => terms["provider_id"] = json!("o"),
            1 => terms["currency_id"] = json!("FOREIGN"),
            2 => terms["duration_days"] = json!(0),
            _ => terms["source"] = Value::Null,
        };
        let before = serde_json::to_value(&w).unwrap();
        let result = plan_artisan_choices(&w, &common::request(), false);
        if case == 0 {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().operations.is_empty());
        }
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}

#[test]
fn zero_cost_retool_and_countdown_only_completion_preserve_authored_terms() {
    let mut w = fixture();
    w.production_data.artisans[0]
        .base
        .extra
        .get_mut("retooling")
        .unwrap()["cost_minor"] = json!("0");
    let result = plan_artisan_choices(&w, &common::request(), false).unwrap();
    assert_eq!(result.operations.len(), 1);
    assert!(result.operations[0].debits.is_empty() && result.operations[0].credits.is_empty());
    w.production_data.artisans = result.enterprises;
    w.production_data.artisans[0]
        .pending_retooling
        .as_mut()
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("ready_date");
    w.production_data.artisans[0]
        .pending_retooling
        .as_mut()
        .unwrap()["remaining_days"] = json!(7);
    let result = plan_artisan_choices(&w, &common::request(), true).unwrap();
    assert_eq!(
        result.enterprises[0].base.method_id,
        ExternalId::from("artisan:tools")
    );
    assert!(result.operations[0].debits.is_empty() && result.operations[0].credits.is_empty());
}

#[test]
fn integral_json_number_duration_and_countdown_follow_source_number_semantics() {
    let mut w = fixture();
    w.production_data.artisans[0]
        .base
        .extra
        .get_mut("retooling")
        .unwrap()["duration_days"] = json!(10.0);
    let result = plan_artisan_choices(&w, &common::request(), false).unwrap();
    assert_eq!(result.operations.len(), 1);
    w.production_data.artisans = result.enterprises;
    let pending = w.production_data.artisans[0]
        .pending_retooling
        .as_mut()
        .unwrap();
    pending.as_object_mut().unwrap().remove("ready_date");
    pending["remaining_days"] = json!(7.0);
    let result = plan_artisan_choices(&w, &common::request(), true).unwrap();
    assert!(result.enterprises[0].pending_retooling.is_none());
}

#[test]
fn profitable_current_method_is_retained_without_retooling_charge() {
    let mut w = fixture();
    w.stockpiles
        .iter_mut()
        .filter(|s| s.goods == Goods::Coal)
        .for_each(|s| s.quantity = 0.4);
    let quotes = vec![
        json!({"province_id":1,"currency_id":"COIN","goods_id":"steel","price":3,"demand":1}),
        json!({"province_id":1,"currency_id":"COIN","goods_id":"iron","price":1,"available":0}),
        json!({"province_id":1,"currency_id":"COIN","goods_id":"coal","price":1,"available":0}),
    ];
    let result = choose_artisan_method(&w, &"a".into(), &quotes, None).unwrap();
    assert_eq!(result.method_id, ExternalId::from("artisan:steel"));
    assert!(result.retooling.is_none());
    assert_eq!(
        serde_json::to_value(result).unwrap(),
        oracle(&w, "artisan_choose", &quotes, false, None)
    );
}
