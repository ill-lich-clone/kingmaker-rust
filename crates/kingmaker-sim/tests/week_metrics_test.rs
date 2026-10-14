#[path = "common/native_week.rs"]
mod common;
use kingmaker_core::{operations::OperationKind, AccountBalance, AccountId, Money, PopId};
use kingmaker_sim::{
    native_week::execute_native_week,
    week_metrics::{collect_week_metrics, freeze_week_summary},
};
use serde_json::{json, Value};

#[test]
fn actual_public_payroll_and_old_wage_debt_reconcile_by_state_currency_and_sector() {
    #[path = "common/budget.rs"]
    mod budget;
    let mut w = budget::world(5000);
    budget::funding(&mut w, 100., 100.);
    let empty = common::empty();
    w.production_data.catalog = empty.production_data.catalog;
    w.market_data.needs = empty.market_data.needs;
    w.mobility_data.reference = empty.mobility_data.reference;
    w.production_data.obligations.push(serde_json::from_value(json!({
        "obligation_id":"old-teacher-wage", "payer_id":"s", "payee_id":"teacher",
        "currency_id":"X", "purpose":"state_service:education", "payee_purpose":"personal_savings",
        "amount_minor":"200", "tax_rate":0.1, "tax_owner_id":"treasury:s", "pop_id":"teacher",
        "budget_state_id":"s", "budget_sector":"education"
    })).unwrap());
    let before = freeze_week_summary(&w).unwrap();
    let request = common::request(&w);
    let receipt = execute_native_week(&mut w, &request, |_| {}).unwrap();
    let metrics = collect_week_metrics(&before, &w, &receipt).unwrap();
    let paid: i128 = w
        .production_data
        .committed_operations
        .iter()
        .filter_map(|r| {
            serde_json::from_value::<kingmaker_core::operations::EconomicOperation>(r.value.clone())
                .ok()
        })
        .filter(|o| {
            matches!(o.kind, OperationKind::Payroll | OperationKind::DebtPayment)
                && o.meta.get("budget_state_id") == Some(&json!("s"))
        })
        .flat_map(|o| o.debits)
        .map(|l| i128::from(l.amount_minor.0))
        .sum();
    assert_eq!(paid, 2200);
    assert_eq!(
        amount(&metrics.rows, "public_service_payroll_paid", None),
        json!("2200")
    );
    for (sector, expected) in [("education", "1200"), ("medicine", "1000")] {
        assert!(metrics
            .rows
            .iter()
            .any(|r| r["scope"] == json!({"state":"s"})
                && r["name"] == "public_service_payroll_paid_by_sector"
                && r["currency_id"] == "X"
                && r["purpose"] == sector
                && r["amount"] == expected));
    }
}

#[test]
fn real_route_and_disabled_procurement_reasons_survive_weekly_metrics_with_ids() {
    #[path = "common/physical_supply.rs"]
    mod physical;
    let mut w = physical::opening();
    w.mobility_data.reference = common::empty().mobility_data.reference;
    w.transport_data.supply_routes.clear();
    for p in &mut w.market_data.territory {
        p.extra.insert("owner_state_id".into(), json!("s"));
    }
    w.production_data.catalog.as_mut().unwrap().methods.push(
        serde_json::from_value(json!({
        "method_id":"raw-iron","kind":"rgo","output_goods_id":"iron","output_unit":"t",
        "status":"ready","weekly_productivity":1,"inputs":[],"staffing":["laborers"]}))
        .unwrap(),
    );
    w.production_data.resource_operations.push(
        serde_json::from_value(json!({"id":"raw",
        "owner_id":"seller","province_id":1,"method_id":"raw-iron","resource_id":"iron",
        "resource_status":"source","land_access":true,"initial_multiplier":1,
        "facility_id":"wh:seller","assignments":[]}))
        .unwrap(),
    );
    let mut facility = w
        .production_data
        .storage_facilities
        .iter()
        .find(|f| f.owner_id == kingmaker_core::ExternalId::from("seller"))
        .unwrap()
        .clone();
    facility.id = "state-wh".into();
    facility.owner_id = "treasury:s".into();
    facility
        .extra
        .insert("state_raw_warehouse".into(), json!(true));
    w.production_data.storage_facilities.push(facility);
    w.production_data
        .policies
        .extra
        .insert("state_budgets".into(), json!({"s":{"raw_procurement":0}}));
    let before = freeze_week_summary(&w).unwrap();
    let request = common::request(&w);
    let receipt = execute_native_week(&mut w, &request, |_| {}).unwrap();
    for reason in [
        "supply_route_or_offer_unavailable",
        "state_procurement_disabled",
    ] {
        assert!(
            receipt.diagnostics.iter().any(|r| r["reason"] == reason),
            "missing actual fixture reason {reason}: {:?}",
            receipt.diagnostics
        );
    }
    let metrics = collect_week_metrics(&before, &w, &receipt).unwrap();
    for reason in [
        "supply_route_or_offer_unavailable",
        "state_procurement_disabled",
    ] {
        assert!(
            metrics.problem_events.iter().any(|r| r["reason"] == reason
                && !r["id"].is_null()
                && r["period"] == request.period),
            "lost actual {reason}"
        );
    }
    assert!(metrics
        .rows
        .iter()
        .all(|r| r["name"] != "dispatch_quantity"));
}
fn amount(rows: &[Value], name: &str, good: Option<&str>) -> Value {
    rows.iter()
        .find(|r| {
            r["scope"] == json!("world")
                && r["name"] == name
                && good.is_none_or(|g| r["goods_id"] == g)
        })
        .unwrap_or_else(|| panic!("metric{name}/{good:?} absent"))
        .get("amount")
        .unwrap()
        .clone()
}
#[test]
fn actual_factory_inputs_output_paid_needs_and_tax_are_measured_without_double_counting() {
    let mut w = common::factory(false);
    let before = freeze_week_summary(&w).unwrap();
    let r = common::request(&w);
    let receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    let snapshot = serde_json::to_value(&w).unwrap();
    let metrics = collect_week_metrics(&before, &w, &receipt).unwrap();
    assert_eq!(
        amount(&metrics.rows, "production", Some("steel")),
        json!(1.)
    );
    assert_eq!(
        amount(&metrics.rows, "production_inputs", Some("iron")),
        json!(1.1)
    );
    assert_eq!(
        amount(&metrics.rows, "production_inputs", Some("coal")),
        json!(0.4)
    );
    assert_eq!(amount(&metrics.rows, "cash_change", None), json!("0"));
    assert_eq!(amount(&metrics.rows, "tax_receipts", None), json!("1"));
    assert_eq!(
        amount(&metrics.rows, "free_declared_job_fte", None),
        json!(0.)
    );
    assert_eq!(
        amount(&metrics.rows, "authored_unit_price", Some("steel")),
        json!(3.)
    );
    assert!(
        amount(&metrics.rows, "consumer_quantity", Some("steel"))
            .as_f64()
            .unwrap()
            > 0.
    );
    assert!(metrics
        .rows
        .iter()
        .any(|r| r["scope"] == json!({"pop_class":"craftsmen"})
            && r["name"] == "recognized_net_income"));
    assert!(metrics
        .rows
        .iter()
        .any(|r| r["scope"] == json!({"province":1}) && r["name"] == "production"));
    assert!(metrics
        .rows
        .iter()
        .any(|r| r["scope"] == json!({"state":"s"}) && r["name"] == "production"));
    assert_eq!(serde_json::to_value(&w).unwrap(), snapshot);
}

#[test]
fn construction_reports_actual_investor_operations_and_current_project_status() {
    #[path = "common/construction.rs"]
    mod construction;
    let mut w = construction::opening(7);
    let before = freeze_week_summary(&w).unwrap();
    let mut r = common::request(&w);
    r.period = "w".into();
    let receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    let metrics = collect_week_metrics(&before, &w, &receipt).unwrap();
    assert!(!metrics.project_operations.is_empty());
    assert_eq!(
        amount(&metrics.rows, "construction_started", None),
        json!(1)
    );
    assert!(metrics
        .construction_summary
        .iter()
        .any(|r| r["status"] == "completed"));
    assert!(metrics
        .project_operations
        .iter()
        .any(|o| o.kind == OperationKind::CapitalistInvestment && !o.debits.is_empty()));
}
#[test]
fn freight_arrival_and_actual_consumption_are_separate_from_new_production() {
    let mut w = common::factory(true);
    let before = freeze_week_summary(&w).unwrap();
    let r = common::request(&w);
    let receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    let first = collect_week_metrics(&before, &w, &receipt).unwrap();
    assert!(first
        .shipment_events
        .iter()
        .any(|e| e["shipment"]["route_id"] == "road"
            && !e["shipment"]["id"].is_null()
            && e["period"] == r.period));
    assert_eq!(
        amount(&first.rows, "dispatch_quantity", Some("steel")),
        json!(1.)
    );
    assert_eq!(
        amount(&first.rows, "in_transit_quantity", Some("steel")),
        json!(1.)
    );
    let before = freeze_week_summary(&w).unwrap();
    let r = common::request(&w);
    let receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    let second = collect_week_metrics(&before, &w, &receipt).unwrap();
    assert_eq!(
        amount(&second.rows, "arrival_quantity", Some("steel")),
        json!(1.)
    );
    assert_eq!(
        amount(&second.rows, "consumer_quantity", Some("steel")),
        json!(1.)
    );
    assert_ne!(
        amount(&second.rows, "production", Some("steel")),
        json!(1.),
        "arrival is not another manufactured unit"
    );
}
#[test]
fn unavailable_need_measurements_remain_explicit_and_altered_week_is_rejected() {
    let mut w = common::empty();
    let before = freeze_week_summary(&w).unwrap();
    let r = common::request(&w);
    let mut receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    let metrics = collect_week_metrics(&before, &w, &receipt).unwrap();
    assert!(metrics.not_measured.iter().any(|s| s.contains("poverty")));
    assert!(metrics.not_measured.iter().any(|s| s.contains("need")));
    receipt.end_revision += 1;
    assert!(collect_week_metrics(&before, &w, &receipt).is_err());
}

#[test]
fn real_dividend_withholding_is_included_in_tax_metrics() {
    let mut w = common::factory(false);
    w.production_data.policies.taxes.insert("s".into(), 0.1);
    let mut aristocrat = w.pops[0].clone();
    aristocrat.id = PopId(99);
    aristocrat.external_id = "ar".into();
    aristocrat.pop_type = "aristocrats".into();
    aristocrat.pop_type_id = "aristocrats".into();
    aristocrat.workforce = 0.;
    aristocrat.employed = 0.;
    aristocrat.unemployed = 0.;
    aristocrat.account_id = None;
    aristocrat.account_status = "absent".into();
    w.pops.push(aristocrat);
    w.accounts.push(AccountBalance {
        id: AccountId(900),
        external_id: "p-author-savings".into(),
        owner_id: "p".into(),
        currency_id: "COIN".into(),
        purpose: "personal_savings".into(),
        balance: Money(1000),
    });
    w.rebuild_account_index().unwrap();
    let prices = w
        .market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap();
    prices.insert("iron".into(), json!(0.1));
    prices.insert("coal".into(), json!(0.1));
    let owner = w.production_data.factories[0]
        .base
        .owner_id
        .clone()
        .unwrap();
    w.mobility_data.production_beneficiaries.push(json!({"owner_id":owner,"currency_id":"COIN","source":"explicit mini ownership","beneficiaries":[{"owner_id":"ar","pop_id":"ar","state_id":"s","purpose":"personal_savings","numerator":"1","denominator":"1"}]}));
    let before = freeze_week_summary(&w).unwrap();
    let r = common::request(&w);
    let receipt = execute_native_week(&mut w, &r, |_| {}).unwrap();
    let dividend = w
        .income_data
        .phase_receipts
        .iter()
        .flat_map(|p| &p.operations)
        .find(|o| o.kind == OperationKind::EnterpriseDividend)
        .unwrap();
    let withheld = dividend.meta["income_by_pop"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            serde_json::from_value::<Money>(r["tax_minor"].clone())
                .unwrap()
                .0
        })
        .sum::<i64>();
    assert!(withheld > 0, "exercise actual withheld dividend tax");
    let metrics = collect_week_metrics(&before, &w, &receipt).unwrap();
    assert_eq!(
        amount(&metrics.rows, "tax_receipts", None),
        json!((1 + withheld).to_string())
    );
}
