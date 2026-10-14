#[path = "common/native_week.rs"]
mod common;
#[path = "common/construction.rs"]
mod construction;
#[path = "common/labor.rs"]
mod labor_fixture;
#[path = "common/production.rs"]
mod production;
#[path = "common/income.rs"]
mod source_snapshot;
use kingmaker_core::*;
use kingmaker_sim::native_week::{apply_native_week, execute_native_week, prepare_native_week};

#[test]
fn cancellation_after_real_production_and_market_keeps_the_original_world() {
    let w = common::factory(false);
    let r = common::request(&w);
    let before = serde_json::to_value(&w).unwrap();
    let mut phases = vec![];
    let result = kingmaker_sim::native_week::try_prepare_native_week(&w, &r, |phase| {
        phases.push(phase.to_owned());
        if phase == "income" {
            Err(kingmaker_sim::operation_session::OperationError(
                "CANCELLED_BY_OPERATOR".into(),
            ))
        } else {
            Ok(())
        }
    });
    assert!(result.is_err());
    assert!(phases.iter().any(|p| p == "production"));
    assert!(phases.iter().any(|p| p == "market"));
    assert_eq!(phases.last().unwrap(), "income");
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}

#[test]
fn historic_week_replay_survives_a_later_legitimate_full_cohort_identity_transfer() {
    let mut w = profession_world();
    let first = common::request(&w);
    let receipt = execute_native_week(&mut w, &first, |_| {}).unwrap();
    let former_child = w.mobility_data.cohort_receipts[0].child_id.clone().unwrap();
    // Explicit later author revocation; the engine releases the existing worker.
    w.production_data.resource_operations[0].land_access = Some(false);
    let second = common::request(&w);
    execute_native_week(&mut w, &second, |_| {}).unwrap();
    assert_eq!(
        w.pops
            .iter()
            .find(|p| p.external_id == former_child)
            .unwrap()
            .unemployed,
        1.
    );
    let r = kingmaker_core::mobility::CohortSplitRequest {
        expected_revision: w.campaign.revision,
        event_id: "later-whole-identity".into(),
        pop_id: former_child.clone(),
        workforce_count: 1,
        target: Default::default(),
    };
    let split = kingmaker_sim::cohort_cycle::execute_pop_split(&mut w, &r).unwrap();
    assert_eq!(split.workforce_moved, 1);
    assert_eq!(
        w.pops
            .iter()
            .find(|p| p.external_id == former_child)
            .unwrap()
            .size,
        0,
        "historical identity remains as a zero-size parent"
    );
    assert_eq!(
        kingmaker_sim::week_history::validate_week_history(&w, &first).unwrap(),
        Some(receipt)
    );
}
#[test]
fn whole_week_calendar_rolls_actual_30_day_year_once_and_second_period_is_distinct() {
    let mut w = common::empty();
    w.campaign.month = 12;
    w.campaign.day = 28;
    let r = common::request(&w);
    let first = execute_native_week(&mut w, &r, |_| {}).unwrap();
    assert_eq!(
        (
            w.campaign.year,
            w.campaign.month,
            w.campaign.day,
            w.campaign.revision
        ),
        (307, 1, 5, 21)
    );
    let next = common::request(&w);
    let second = execute_native_week(&mut w, &next, |_| {}).unwrap();
    assert_eq!(
        (
            w.campaign.year,
            w.campaign.month,
            w.campaign.day,
            w.campaign.revision
        ),
        (307, 1, 12, 22)
    );
    assert_ne!(first.boundary.event_id, second.boundary.event_id);
    assert_eq!(execute_native_week(&mut w, &r, |_| {}).unwrap(), first);
}
#[test]
fn failed_phase_or_changed_prepared_world_never_publishes_partial_week() {
    let mut w = common::empty();
    let r = common::request(&w);
    let p = prepare_native_week(&w, &r, |_| {}).unwrap();
    w.campaign.day = 2;
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_native_week(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut broken = common::empty();
    broken.production_data.catalog = None;
    let before = serde_json::to_value(&broken).unwrap();
    assert!(execute_native_week(&mut broken, &r, |_| {}).is_err());
    assert_eq!(serde_json::to_value(&broken).unwrap(), before);
}
#[test]
fn real_factory_week_pays_labor_consumes_inputs_and_sells_actual_output_without_seeded_offer() {
    let mut w = common::factory(false);
    let r = common::request(&w);
    let money = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    assert!(w.market_data.sale_offers.is_empty());
    let receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    assert_eq!(
        w.production_data.phase_receipts[0]
            .outputs
            .iter()
            .map(|o| o.quantity)
            .sum::<f64>(),
        1.
    );
    assert_eq!(
        w.stockpiles
            .iter()
            .find(|s| s.external_id == ExternalId::from("iron"))
            .unwrap()
            .quantity,
        0.
    );
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), money);
    assert!(w
        .market_data
        .phase_receipts
        .iter()
        .flat_map(|p| &p.operations)
        .any(|o| o.kind == kingmaker_core::operations::OperationKind::Purchase));
    assert!(receipt.invariants.production_required_passed());
    assert_eq!(
        receipt.phase_order,
        vec![
            "storage_capacity",
            "arrivals",
            "mobility",
            "budget_opening",
            "investments",
            "artisan_opening",
            "service_payroll",
            "maintenance",
            "early_food",
            "builder_payroll",
            "production",
            "construction",
            "market_offers",
            "market",
            "supply_offers",
            "food_reserves",
            "supply",
            "state_procurement",
            "income",
            "labor_history",
            "closing"
        ]
    );
    assert_eq!(receipt.timings.len(), receipt.phase_order.len());
}
#[test]
fn actual_output_is_shipped_then_next_week_arrives_consumes_and_recognizes_paid_income() {
    let mut w = common::factory(true);
    let r = common::request(&w);
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    assert_eq!(w.mobility_data.shipments.len(), 1);
    assert_eq!(
        w.mobility_data.shipments[0]["remaining_quantity"].as_f64(),
        Some(1.)
    );
    assert!(w
        .stockpiles
        .iter()
        .filter(|s| s.owner_id == ExternalId::from("buyer"))
        .all(|s| s.quantity == 0.));
    let next = common::request(&w);
    execute_native_week(&mut w, &next, |_| {}).unwrap();
    assert_eq!(
        w.mobility_data.shipments[0]["remaining_quantity"].as_f64(),
        Some(0.)
    );
    assert!(w
        .market_data
        .phase_receipts
        .iter()
        .filter(|p| p.request.period == next.period)
        .flat_map(|p| &p.operations)
        .any(
            |o| o.kind == kingmaker_core::operations::OperationKind::Purchase
                && o.meta.get("consumed").and_then(serde_json::Value::as_bool) == Some(true)
        ));
    assert!(w
        .income_data
        .producer_income
        .iter()
        .any(|p| p["owner_id"] == serde_json::json!("o")
            && p["revenue_minor"] == serde_json::json!("300")));
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
}
#[test]
fn actual_investment_builder_payroll_and_construction_complete_in_the_whole_week() {
    let mut w = construction::opening(7);
    let mut r = common::request(&w);
    r.period = "w".into();
    let initial = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    assert!(w
        .production_data
        .construction_projects
        .iter()
        .any(|p| p.status == "completed"));
    assert_eq!(w.production_data.factories.len(), 1);
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), initial);
    assert!(w
        .production_data
        .committed_operations
        .iter()
        .any(|o| o.value["type"] == serde_json::json!("capitalist_investment")));
}
#[test]
fn closing_pays_actual_artisan_profit_before_actual_producer_dividend_in_source_order() {
    use serde_json::json;
    let mut w = common::factory(false);
    w.accounts
        .push(production::account(100, "p", "personal_savings", 10000));
    w.rebuild_account_index().unwrap();
    let configure = |w: &mut WorldState| {
        let mut artist = w.pops[0].clone();
        artist.id = PopId(10);
        artist.external_id = "artist".into();
        artist.pop_type_id = "artisans".into();
        artist.pop_type = "artisans".into();
        w.pops.push(artist);
        let mut ar = w.pops[0].clone();
        ar.id = PopId(11);
        ar.external_id = "ar".into();
        ar.pop_type_id = "aristocrats".into();
        ar.pop_type = "aristocrats".into();
        ar.workforce = 0.;
        ar.employed = 0.;
        w.pops.push(ar);
        w.accounts
            .push(production::account(200, "artist", "operating_capital", 0));
        w.rebuild_account_index().unwrap();
        w.production_data.catalog.as_mut().unwrap().goods.push(
            serde_json::from_value(
                json!({"goods_id":"clothing","kind":"processed_good","unit":"t"}),
            )
            .unwrap(),
        );
        w.production_data.catalog.as_mut().unwrap().goods.push(
            serde_json::from_value(json!({"goods_id":"fibers","kind":"raw_resource","unit":"t"}))
                .unwrap(),
        );
        let mut m = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
        m.method_id = "artisan:clothing".into();
        m.kind = "artisan".into();
        m.output_goods_id = "clothing".into();
        m.inputs = serde_json::from_value(json!([{"goods_id":"fibers","amount":1}])).unwrap();
        m.staffing = vec!["artisans".into()];
        w.production_data.catalog.as_mut().unwrap().methods.push(m);
        w.production_data.artisans.push(serde_json::from_value(json!({"id":"artist-work","owner_id":"artist","province_id":1,"method_id":"artisan:clothing","facility_id":"artist-wh","people_slots":1,"income_policy":{"source":"author","cash_reserve_minor":"0","payout_fraction":1},"assignments":[{"pop_id":"artist","workers_count":1,"time_fraction":1,"employment_id":"artist-job"}]})).unwrap());
        w.production_data.storage_facilities.push(serde_json::from_value(json!({"id":"artist-wh","owner_id":"artist","province_id":1,"capacity":10,"unit":"space","status":"source","source":"author","goods":{"clothing":{"unit":"t","space_per_unit":1},"fibers":{"unit":"t","space_per_unit":1}}})).unwrap());
        let mut raw = w.stockpiles[0].clone();
        raw.id = 300;
        raw.external_id = "artist-input".into();
        raw.owner_id = "artist".into();
        raw.goods = Goods::from_external("fibers".into());
        raw.quantity = 1.;
        raw.currency_id = Some("COIN".into());
        raw.facility_id = Some("artist-wh".into());
        w.stockpiles.push(raw);
        w.market_data.needs.as_mut().unwrap().profiles[0].norms.push(serde_json::from_value(json!({"need_id":"clothing","goods_id":"clothing","H":1,"period":"week","status":"known","quantity_status":"authored","quantity_per_person_week":1,"unit":"t","allocation_rule":"none"})).unwrap());
        w.market_data
            .price_calibration
            .as_mut()
            .unwrap()
            .prices_by_currency
            .get_mut("COIN")
            .unwrap()
            .insert("clothing".into(), json!(3));
        w.market_data
            .price_calibration
            .as_mut()
            .unwrap()
            .prices_by_currency
            .get_mut("COIN")
            .unwrap()
            .insert("fibers".into(), json!(1));
        w.mobility_data.production_beneficiaries = vec![
            json!({"owner_id":"o","currency_id":"COIN","source":"author ownership","beneficiaries":[{"owner_id":"ar","pop_id":"ar","purpose":"personal_savings","state_id":"s","numerator":"1","denominator":"1"}]}),
        ];
    };
    configure(&mut w);
    let request = common::request(&w);
    execute_native_week(&mut w, &request, |_| {}).unwrap();
    let p = w
        .income_data
        .phase_receipts
        .iter()
        .find(|p| p.stage == kingmaker_core::income::IncomeStage::ClosingIncome)
        .unwrap();
    let artisan = p
        .operations
        .iter()
        .position(|o| {
            o.event_id
                .as_str()
                .is_some_and(|s| s.starts_with("artisan-income:"))
        })
        .expect("actual artisan sale must pay profit");
    let producer = p
        .operations
        .iter()
        .position(|o| o.kind == kingmaker_core::operations::OperationKind::EnterpriseDividend)
        .expect("actual factory must pay dividend");
    assert!(
        artisan < producer,
        "Source closes artisan payments before producer dividends"
    );
}
#[test]
fn failure_at_closing_income_keeps_the_actual_initial_world_and_reports_the_phase() {
    use serde_json::json;
    let mut w = common::factory(false);
    w.accounts
        .push(production::account(100, "p", "personal_savings", 10000));
    w.rebuild_account_index().unwrap();
    let mut ar = w.pops[0].clone();
    ar.id = PopId(7);
    ar.external_id = "ar".into();
    ar.pop_type_id = "aristocrats".into();
    ar.pop_type = "aristocrats".into();
    ar.workforce = 0.;
    ar.employed = 0.;
    w.pops.push(ar);
    w.mobility_data.production_beneficiaries = vec![
        json!({"owner_id":"o","currency_id":"COIN","source":"bad author input","beneficiaries":[{"owner_id":"ar","pop_id":"ar","purpose":"personal_savings","state_id":"s","numerator":"2","denominator":"1"}]}),
    ];
    let r = common::request(&w);
    let initial = serde_json::to_value(&w).unwrap();
    let mut phases = vec![];
    let result = execute_native_week(&mut w, &r, |p| phases.push(p.to_owned()));
    assert!(result.is_err());
    assert_eq!(phases.last().map(String::as_str), Some("income"));
    assert_eq!(serde_json::to_value(&w).unwrap(), initial);
}
#[test]
fn real_factory_whole_week_money_and_physical_output_match_actual_source_turn() {
    use serde_json::json;
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    for instant in [false, true] {
        let mut w = common::factory(false);
        w.market_data.parameters.as_mut().unwrap().instant_food = Some(instant);
        let mut r = common::request(&w);
        r.policies.priorities = vec![
            "basic".into(),
            "production_inputs".into(),
            "everyday".into(),
            "luxury".into(),
        ];
        let mut snapshot = source_snapshot::snapshot(&w);
        snapshot["campaign"]["campaign_id"] = json!(w.campaign.campaign_id);
        snapshot["registry"]["completedConstruction"] =
            json!(w.production_data.completed_construction);
        snapshot["registry"]["completedExpansions"] = json!(w.production_data.expansions);
        snapshot["registry"]["reference"] = json!(w.mobility_data.reference);
        snapshot["registry"]["foundationBudgetPolicy"] =
            json!(w.budget_data.foundation_budget_policy);
        snapshot["registry"]["constructionCatalog"] =
            json!(w.construction_data.construction_catalog);
        snapshot["registry"]["constructionCatalogByCurrency"] =
            json!(w.construction_data.construction_catalog_by_currency);
        let mut child = Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/tests/week-oracle.cjs"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(json!({"snapshot":snapshot,"period":r.period,"policies":r.policies,"profile":r.market_profile}).to_string().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let source: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        execute_native_week(&mut w, &r, |_| {}).unwrap();
        let mut expected = std::collections::BTreeMap::new();
        for a in source["accounts"].as_array().unwrap() {
            expected.insert(
                (
                    a["owner_id"].to_string(),
                    a["currency_id"].to_string(),
                    a["purpose"].as_str().unwrap().to_owned(),
                ),
                a["amount_minor"].as_str().unwrap().parse::<i64>().unwrap(),
            );
        }
        let actual = w
            .accounts
            .iter()
            .map(|a| {
                (
                    (
                        a.owner_id.json().to_string(),
                        a.currency_id.json().to_string(),
                        a.purpose.clone(),
                    ),
                    a.balance.0,
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            actual, expected,
            "Source stocks {}; offers {}; priorities {}",
            source["stocks"], source["offers"], source["priorities"]
        );
        let source_steel = source["stocks"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["goods_id"] == json!("steel"))
            .map(|s| s["amount"].as_f64().unwrap())
            .sum::<f64>();
        let native_steel = w
            .stockpiles
            .iter()
            .filter(|s| s.goods == Goods::Steel)
            .map(|s| s.quantity)
            .sum::<f64>();
        assert!((source_steel - native_steel).abs() < 1e-9);
    }
}
#[test]
fn native_week_retains_the_original_unported_source_rows_and_shared_source_identity() {
    use std::sync::Arc;
    let mut w = common::empty();
    let mut source = SourceSnapshot::default();
    source.campaign_id = "original-source".into();
    source.tables.insert(
        "unported/author".into(),
        vec![StoredRecord {
            table_name: "unported/author".into(),
            id_key: "[\"number\",7]".into(),
            external_id_json: Some("7".into()),
            ordinal: 12,
            sequence: 14,
            payload: "{\"author_formula\":\"keep exactly\"}".into(),
        }],
    );
    let source = Arc::new(source);
    w.source = Some(source.clone());
    let r = common::request(&w);
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    assert!(Arc::ptr_eq(w.source.as_ref().unwrap(), &source));
    assert_eq!(w.source.as_ref().unwrap().tables, source.tables);
}
#[test]
fn explicit_empty_market_priority_setting_is_preserved_in_the_whole_week() {
    let mut w = common::factory(false);
    let mut r = common::request(&w);
    r.market_priorities = Some(vec![]);
    execute_native_week(&mut w, &r, |_| {}).unwrap();
    let closing = w
        .market_data
        .phase_receipts
        .iter()
        .find(|p| p.request.stage == kingmaker_core::market::MarketStage::ClosingMarket)
        .unwrap();
    assert!(closing.effective_priorities.is_empty());
    assert!(closing
        .operations
        .iter()
        .all(|o| o.kind != kingmaker_core::operations::OperationKind::Purchase));
    assert_eq!(
        w.account_by_address(&AccountAddress {
            owner_id: "p".into(),
            currency_id: "COIN".into(),
            purpose: "personal_savings".into()
        })
        .unwrap()
        .balance,
        Money(9)
    );
}
#[test]
fn whole_week_full_profession_transfer_fills_the_real_raw_job_without_losing_people_or_money() {
    let mut w = profession_world();
    check_profession_week(&mut w);
}

fn profession_world() -> WorldState {
    use serde_json::json;
    let mut w = labor_fixture::world(100000);
    w.pops.truncate(1);
    w.pops[0].workforce = 1.;
    w.pops[0].size = 2;
    w.pops[0].employed = 0.;
    w.pops[0].unemployed = 1.;
    w.market_data
        .needs
        .as_mut()
        .unwrap()
        .profile_refs
        .retain(|p| p.pop_id == ExternalId::from("a"));
    w.production_data.catalog.as_mut().unwrap().methods[0].staffing = vec!["laborers".into()];
    w.production_data.catalog.as_mut().unwrap().methods[0].method_id = "rgo:iron".into();
    w.production_data.catalog.as_mut().unwrap().methods[0].output_goods_id = "iron".into();
    w.production_data.catalog.as_mut().unwrap().goods.push(
        serde_json::from_value(json!({"goods_id":"iron","kind":"raw_resource","unit":"t"}))
            .unwrap(),
    );
    w.production_data.resource_operations[0].resource_id = "iron".into();
    w.production_data.resource_operations[0].base.method_id = "rgo:iron".into();
    for f in &mut w.production_data.storage_facilities {
        let mapping = f.goods["grain"].clone();
        f.goods.insert("iron".into(), mapping);
    }
    w.production_data.storage_facilities.push(serde_json::from_value(json!({"id":"wh","owner_id":"o","province_id":1,"capacity":1000,"unit":"space","status":"source","source":"author","goods":{"iron":{"unit":"t","space_per_unit":1}}})).unwrap());
    w.mobility_data.reference = Some(
        json!({"professions":[{"pop_type_id":"laborers","transition_requirements":{"kind":"funded_vacancy","source":"author"}}]}),
    );
    w.labor_data.pop_mobility = Some(
        json!({"unemployment":{"a":[{"workers":1,"weeks":1}]},"remainders":{"a":{"profession":0.9}}}),
    );
    w
}

fn check_profession_week(w: &mut WorldState) {
    let people = w.pops.iter().map(|p| u64::from(p.size)).sum::<u64>();
    let money = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let r = common::request(&w);
    execute_native_week(w, &r, |_| {}).unwrap();
    assert_eq!(w.mobility_data.profession_receipts[0].changed_workforce, 1);
    assert_eq!(
        w.pops.iter().map(|p| u64::from(p.size)).sum::<u64>(),
        people
    );
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), money);
    assert!(w
        .pops
        .iter()
        .any(|p| p.pop_type_id == ExternalId::from("laborers") && p.workforce == 1.));
    assert_eq!(
        w.production_data.phase_receipts[0]
            .outputs
            .iter()
            .map(|p| p.quantity)
            .sum::<f64>(),
        1.,
        "planned {:?}; assignments {:?}",
        w.production_data.phase_receipts[0].planned_outputs,
        w.production_data.resource_operations[0].base.assignments
    );
}

#[test]
fn profession_transfer_preserves_and_consumes_the_donors_real_opening_stock() {
    let mut w = profession_world();
    let mut own = w.stockpiles[0].clone();
    own.id = 777;
    own.external_id = "own-grain".into();
    own.owner_id = "a".into();
    own.province_id = ProvinceId(1);
    own.facility_id = Some("wh:a".into());
    own.quantity = 1.;
    w.stockpiles.push(own);
    check_profession_week(&mut w);
    assert!(w.mobility_data.opening_receipts[0]
        .operations
        .iter()
        .any(|o| !o.stock_changes.is_empty()));
    assert!(w
        .market_data
        .phase_receipts
        .iter()
        .flat_map(|p| &p.operations)
        .any(|o| o.kind == kingmaker_core::operations::OperationKind::Purchase));
}
