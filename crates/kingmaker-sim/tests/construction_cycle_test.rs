#[path = "common/construction.rs"]
mod common;
#[path = "common/income.rs"]
mod income_common;
use kingmaker_core::*;
use kingmaker_sim::construction_cycle::{
    apply_construction, execute_construction, prepare_construction,
};
use serde_json::json;
#[test]
fn actual_financing_paid_labor_materials_fee_reserve_and_refunds_complete_real_factory() {
    let mut w = common::ready(7);
    let people = w.pops.iter().map(|p| u64::from(p.size)).sum::<u64>();
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let receipt = execute_construction(&mut w, &common::request()).unwrap();
    let project = serde_json::to_value(&w.production_data.construction_projects[0]).unwrap();
    assert_eq!(project["status"], json!("completed"));
    assert_eq!(project["elapsed_days"], json!(7));
    assert_eq!(project["labor_fte_days"].as_f64().unwrap(), 7.);
    assert_eq!(project["paid_wages_minor"], json!("10"));
    assert_eq!(project["refunded_minor"], json!("5000"));
    assert_eq!(common::cash(&w, "builder", "operating_capital"), 990);
    assert_eq!(
        common::cash(
            &w,
            "enterprise:private-project:offer",
            "employer_operating_capital"
        ),
        4000
    );
    assert_eq!(common::cash(&w, "a", "personal_savings"), 1500);
    assert_eq!(common::cash(&w, "b", "personal_savings"), 3500);
    assert_eq!(w.production_data.factories.len(), 1);
    let f = &w.production_data.factories[0];
    assert_eq!(f.status.as_deref(), Some("awaiting_staff"));
    assert_eq!(f.people_slots, Some(40000.));
    assert!(f.base.assignments.is_empty());
    assert_eq!(f.base.extra["capital_value_minor"], json!("5000"));
    assert_eq!(w.stockpiles[0].quantity, 0.);
    assert_eq!(
        w.pops.iter().map(|p| u64::from(p.size)).sum::<u64>(),
        people
    );
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    assert!(receipt.invariants.all_required_passed());
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(
        execute_construction(&mut w, &common::request()).unwrap(),
        receipt
    );
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn genuine_missing_material_or_unpaid_labor_waits_without_free_progress() {
    {
        let mut w = common::ready_with(30, |w| w.stockpiles[0].quantity = 0.);
        let stocks = w.stockpiles.clone();
        let receipt = execute_construction(&mut w, &common::request()).unwrap();
        assert_eq!(w.stockpiles, stocks);
        assert!(receipt
            .diagnostics
            .iter()
            .any(|d| d["reason"] == "construction_materials_unavailable"));
        assert_ne!(
            w.production_data.construction_projects[0].status,
            "completed"
        );
    }
}
#[test]
fn zero_cash_paid_fte_is_zero_and_construction_waits() {
    let mut w = common::ready_with(30, |w| {
        w.accounts[2].balance = Money(0);
        w.accounts[3].balance = Money(0);
        w.mobility_data.capitalist_entities.clear();
        w.construction_data.investment_opportunities.clear();
        w.accounts.push(kingmaker_core::AccountBalance {
            id: AccountId(9),
            external_id: "empty-project".into(),
            owner_id: "project:p".into(),
            currency_id: "COIN".into(),
            purpose: "construction_escrow".into(),
            balance: Money(0),
        });
        w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"p","kind":"factory","owner_id":"project:p","province_id":1,"region_id":"r","status":"funded","terms":{"currency_id":"COIN","budget_minor":"1000"},"construction":{"status":"ready","source":"fixture","currency_id":"COIN","duration_days":30,"labor_required_fte_days":7,"cash_cost_basis":"includes_wages","provider_id":"builder","cost_minor":"1000","materials":[]},"assignments":[{"employment_id":"builder-empty","contract_id":"empty-c","pop_id":"p","workers_count":1,"time_fraction":1}]})).unwrap());
        w.production_data.contracts.push(serde_json::from_value(json!({"contract_id":"empty-c","employer_entity_id":"project:p","currency_id":"COIN","payment_purpose":"construction_escrow","weekly_wage_rate":{"numerator":"10","denominator":"1"}})).unwrap());
        w.rebuild_account_index().unwrap();
    });
    let receipt = execute_construction(&mut w, &common::request()).unwrap();
    assert_eq!(
        w.production_data.construction_projects[0].status,
        "awaiting_labor"
    );
    assert!(receipt
        .diagnostics
        .iter()
        .any(|d| d["reason"] == "construction_paid_labor_unavailable"));
    assert!(w.production_data.factories.is_empty());
}
#[test]
fn stale_construction_or_lost_paid_labor_never_spends_fee_or_materials() {
    for case in 0..3 {
        let mut w = common::ready(7);
        let prepared = prepare_construction(&w, &common::request()).unwrap();
        match case {
            0 => w.production_data.storage_facilities[0].owner_id = "other".into(),
            1 => w.stockpiles[0].quantity += 1.,
            _ => w
                .construction_data
                .phase_receipts
                .retain(|p| p.stage != kingmaker_core::construction::ConstructionStage::Payroll),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(apply_construction(&mut w, prepared).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn completed_factory_snapshot_preserves_asset_investors_and_exact_history() {
    let mut w = common::ready(7);
    let receipt = execute_construction(&mut w, &common::request()).unwrap();
    let mut loaded: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    loaded.rebuild_account_index().unwrap();
    assert_eq!(
        loaded.production_data.factories,
        w.production_data.factories
    );
    assert_eq!(
        execute_construction(&mut loaded, &common::request()).unwrap(),
        receipt
    );
}
#[test]
fn completed_factory_hires_and_produces_at_the_next_ordinary_opening() {
    let mut w = common::ready(7);
    execute_construction(&mut w, &common::request()).unwrap();
    let r = kingmaker_core::labor::LaborRequest {
        expected_revision: 20,
        period: "next".into(),
        policies: Default::default(),
    };
    let hired = kingmaker_sim::labor_cycle::execute_local_labor(&mut w, &r).unwrap();
    assert_eq!(hired.hired_fte, 1., "{:?}", hired.blocked);
    assert_eq!(
        w.production_data.factories[0].base.assignments[0].pop_id,
        ExternalId::from("craft")
    );
    kingmaker_sim::budget_cycle::execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "next".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    let mut pr = kingmaker_core::production::ProductionRequest {
        expected_revision: 20,
        period: "next".into(),
        policies: Default::default(),
        profile: Default::default(),
        maintenance_receipts: vec![],
    };
    pr.policies = w.production_data.policies.clone();
    let produced = kingmaker_sim::production::execute_production_cycle(&mut w, &pr).unwrap();
    assert!(
        w.stockpiles.iter().any(|s| s.owner_id
            == ExternalId::from("enterprise:private-project:offer")
            && s.goods == Goods::Steel
            && s.quantity > 0.),
        "{:?}",
        produced
    );
}
#[test]
fn ordinary_completion_matches_the_current_source_rules() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut w = common::ready(7);
    let mut snapshot = income_common::snapshot(&w);
    snapshot["campaign_id"] = json!(w.campaign.campaign_id);
    snapshot["campaign"]["campaign_id"] = json!(w.campaign.campaign_id);
    snapshot["appliedEventIds"] = json!(w
        .production_data
        .committed_operations
        .iter()
        .map(|o| &o.event_id)
        .collect::<Vec<_>>());
    let registry = &mut snapshot["registry"];
    registry["constructionProjects"] = json!(w.production_data.construction_projects);
    registry["projectInvestments"] = json!(w.mobility_data.project_investments);
    registry["completedConstruction"] = json!(w.production_data.completed_construction);
    registry["completedExpansions"] = json!(w.production_data.expansions);
    registry["institutionLevelsPrepared"] = json!({"version":1});
    let paid = &w
        .construction_data
        .phase_receipts
        .iter()
        .find(|p| p.stage == kingmaker_core::construction::ConstructionStage::Payroll)
        .unwrap()
        .operations;
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/construction-oracle.cjs"
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
        .write_all(
            json!({"action":"progress","snapshot":snapshot,"period":"w","paidLabor":paid})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let actual = execute_construction(&mut w, &common::request()).unwrap();
    let mut source_ops: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(expected["operations"].clone()).unwrap();
    for op in &mut source_ops {
        if let Some(n) = op
            .meta
            .get("labor_fte_days")
            .and_then(serde_json::Value::as_f64)
        {
            op.meta.insert("labor_fte_days".into(), json!(n));
        }
    }
    assert_eq!(
        &actual.operations[..actual.operations.len() - 1],
        source_ops
    );
    let mut projects: Vec<kingmaker_core::production::ConstructionProjectLabor> =
        serde_json::from_value(expected["registry"]["constructionProjects"].clone()).unwrap();
    for p in &mut projects {
        if let Some(n) = p
            .extra
            .get("labor_fte_days")
            .and_then(serde_json::Value::as_f64)
        {
            p.extra.insert("labor_fte_days".into(), json!(n));
        }
    }
    assert_eq!(w.production_data.construction_projects, projects);
    let factories: Vec<kingmaker_core::production::FactoryEnterprise> =
        serde_json::from_value(expected["registry"]["factoryEnterprises"].clone()).unwrap();
    assert_eq!(w.production_data.factories, factories);
}
#[test]
fn two_real_periods_consume_materials_and_paid_labor_once() {
    let mut w = common::ready(14);
    execute_construction(&mut w, &common::request()).unwrap();
    assert_eq!(
        w.production_data.construction_projects[0].status,
        "building"
    );
    assert_eq!(w.stockpiles[0].quantity, 0.);
    let r = kingmaker_core::construction::ConstructionRequest {
        expected_revision: 20,
        period: "next".into(),
        policies: Default::default(),
    };
    kingmaker_sim::budget_cycle::execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "next".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    kingmaker_sim::investments::execute_automatic_investments(&mut w, &r).unwrap();
    kingmaker_sim::construction_payroll::execute_construction_payroll(&mut w, &r).unwrap();
    kingmaker_sim::production::execute_production_cycle(
        &mut w,
        &kingmaker_core::production::ProductionRequest {
            expected_revision: 20,
            period: "next".into(),
            policies: Default::default(),
            profile: Default::default(),
            maintenance_receipts: vec![],
        },
    )
    .unwrap();
    let result = execute_construction(&mut w, &r).unwrap();
    assert_eq!(
        w.production_data.construction_projects[0].status,
        "completed"
    );
    assert_eq!(
        w.production_data.construction_projects[0].extra["paid_wages_minor"],
        json!("10")
    );
    assert!(!result
        .operations
        .iter()
        .any(|o| o.kind == kingmaker_core::operations::OperationKind::ConstructionMaterials));
    assert_eq!(w.stockpiles[0].quantity, 0.);
}
#[test]
fn missing_fee_reserve_configuration_or_parameters_waits_without_free_factory() {
    for case in 0..4 {
        let mut w = common::ready(7);
        match case {
            0 => {
                w.accounts
                    .iter_mut()
                    .find(|a| a.purpose == "construction_escrow")
                    .unwrap()
                    .balance = Money(0);
            }
            1 => {
                w.accounts
                    .iter_mut()
                    .find(|a| a.purpose == "factory_startup_reserve")
                    .unwrap()
                    .balance = Money(0);
            }
            2 => {
                w.production_data.construction_projects[0]
                    .extra
                    .get_mut("factory_configuration")
                    .unwrap()["people_slots"] = json!(39999);
            }
            _ => {
                w.production_data.construction_projects[0]
                    .extra
                    .get_mut("construction")
                    .unwrap()["duration_days"] = json!(0);
            }
        }
        let accounts = w.accounts.clone();
        let result = execute_construction(&mut w, &common::request()).unwrap();
        assert!(!result.diagnostics.is_empty());
        assert!(w.production_data.factories.is_empty());
        assert_eq!(w.accounts, accounts);
    }
}
#[test]
fn actual_builder_payroll_is_recognized_by_income_with_required_construction_history() {
    let mut w = common::opening(7);
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
    kingmaker_sim::investments::execute_automatic_investments(&mut w, &common::request()).unwrap();
    kingmaker_sim::construction_payroll::execute_construction_payroll(&mut w, &common::request())
        .unwrap();
    kingmaker_sim::production::execute_production_cycle(
        &mut w,
        &kingmaker_core::production::ProductionRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
            profile: Default::default(),
            maintenance_receipts: vec![],
        },
    )
    .unwrap();
    execute_construction(&mut w, &common::request()).unwrap();
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
    income_common::finish_supply(&mut w, "w");
    let receipt = kingmaker_sim::income_cycle::execute_income_closing(&mut w, &r, &basis).unwrap();
    assert!(receipt.invariants.all_required_passed());
    for case in 0..3 {
        let mut altered = w.clone();
        match case {
            0 => altered
                .construction_data
                .phase_receipts
                .retain(|p| p.stage != kingmaker_core::construction::ConstructionStage::Payroll),
            1 => altered
                .production_data
                .committed_operations
                .retain(|o| o.value["type"] != json!("capitalist_investment")),
            _ => {
                altered
                    .construction_data
                    .phase_receipts
                    .iter_mut()
                    .find(|p| p.stage == kingmaker_core::construction::ConstructionStage::Progress)
                    .unwrap()
                    .registry_changes["projects"] = json!([])
            }
        }
        let before = serde_json::to_value(&altered).unwrap();
        assert!(
            kingmaker_sim::income_cycle::execute_income_closing(&mut altered, &r, &basis).is_err()
        );
        assert_eq!(serde_json::to_value(&altered).unwrap(), before);
    }
}
#[test]
fn authored_configuration_cannot_overwrite_actual_investor_rights_or_net_capital() {
    let mut w = common::ready(7);
    let config = w.production_data.construction_projects[0]
        .extra
        .get_mut("factory_configuration")
        .unwrap();
    config["shares"] = json!([]);
    config["capital_value_minor"] = json!("0");
    config["level"] = json!(99);
    config["provenance"] = json!({"project_id":"other"});
    config["author_custom_field"] = json!("preserved");
    execute_construction(&mut w, &common::request()).unwrap();
    let asset = serde_json::to_value(&w.production_data.factories[0]).unwrap();
    assert_eq!(
        asset["shares"],
        w.production_data.construction_projects[0].extra["shares"]
    );
    assert_eq!(asset["capital_value_minor"], json!("5000"));
    assert_eq!(asset["level"], json!(1));
    assert_eq!(
        asset["provenance"]["project_id"],
        json!("private-project:offer")
    );
    assert_eq!(asset["author_custom_field"], json!("preserved"));
}
