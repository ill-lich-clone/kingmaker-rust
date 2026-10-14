#[path = "common/production.rs"]
mod common;
use kingmaker_core::{construction::ConstructionRequest, operations::OperationKind, *};
use kingmaker_sim::{
    budget_cycle::execute_budget_cycle,
    construction_payroll::{
        apply_construction_payroll, execute_construction_payroll, prepare_construction_payroll,
    },
    investments::execute_automatic_investments,
};
use serde_json::json;
fn request() -> ConstructionRequest {
    ConstructionRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    }
}
fn opening(cash: i64, debt: i64) -> WorldState {
    let mut w = common::world();
    w.production_data.factories.clear();
    w.production_data.contracts.clear();
    w.pops[0].pop_type_id = "farmers".into();
    w.pops[0].pop_type = "farmers".into();
    w.pops[0].size = 2;
    w.pops[0].workforce = 2.;
    w.pops[0].employed = 2.;
    w.accounts
        .push(common::account(3, "project:p", "construction_escrow", cash));
    w.rebuild_account_index().unwrap();
    w.production_data.policies.taxes.insert("s".into(), 0.1);
    w.production_data.contracts.push(serde_json::from_value(json!({"contract_id":"build-c","employer_entity_id":"project:p","currency_id":"COIN","payment_purpose":"construction_escrow","weekly_wage_rate":{"numerator":"10","denominator":"1"}})).unwrap());
    w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"p","kind":"factory","owner_id":"project:p","province_id":1,"region_id":"r","status":"funded","construction":{"status":"ready","source":"fixture","currency_id":"COIN","duration_days":30,"labor_required_fte_days":7,"materials":[{"goods_id":"iron","amount":1}]},"assignments":[{"employment_id":"builder-e","contract_id":"build-c","pop_id":"p","workers_count":2,"time_fraction":1}]})).unwrap());
    w.stockpiles[0].owner_id = "project:p".into();
    w.stockpiles[0].quantity = 1.;
    if debt > 0 {
        w.production_data.obligations.push(serde_json::from_value(json!({"obligation_id":"old-builder-debt","payer_id":"project:p","payee_id":"p","currency_id":"COIN","purpose":"construction_escrow","amount_minor":debt.to_string(),"tax_rate":0,"payee_purpose":"personal_savings"})).unwrap());
    }
    w
}
fn fixture(cash: i64) -> WorldState {
    let mut w = opening(cash, 0);
    execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    execute_automatic_investments(&mut w, &request()).unwrap();
    w
}
fn money(w: &WorldState, owner: &str, purpose: &str) -> i64 {
    w.accounts
        .iter()
        .filter(|a| a.owner_id == ExternalId::from(owner) && a.purpose == purpose)
        .map(|a| a.balance.0)
        .sum()
}
#[test]
fn real_full_partial_zero_builder_payroll_explains_tax_net_paid_fte_and_exact_replay() {
    for (cash, gross, tax, net, fte) in [(100, 10, 1, 9, 1.), (5, 5, 1, 4, 0.5), (0, 0, 0, 0, 0.)] {
        let mut w = fixture(cash);
        let before = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
        let stocks = w.stockpiles.clone();
        let receipt = execute_construction_payroll(&mut w, &request()).unwrap();
        let op = receipt
            .operations
            .iter()
            .find(|op| op.kind == OperationKind::Payroll)
            .unwrap();
        assert_eq!(op.meta["paid_gross_minor"], json!(gross.to_string()));
        assert_eq!(op.meta["tax_minor"], json!(tax.to_string()));
        assert_eq!(op.meta["net_minor"], json!(net.to_string()));
        assert_eq!(op.meta["actual_fte"], json!(fte));
        assert_eq!(money(&w, "project:p", "construction_escrow"), cash - gross);
        assert_eq!(money(&w, "p", "personal_savings"), net);
        assert_eq!(money(&w, "treasury:s", "operating_capital"), tax);
        assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), before);
        assert_eq!(w.stockpiles, stocks);
        assert!(receipt.invariants.all_required_passed());
        assert!(w.production_data.obligations.is_empty());
        let before = serde_json::to_value(&w).unwrap();
        assert_eq!(
            execute_construction_payroll(&mut w, &request()).unwrap(),
            receipt
        );
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn budget_pays_prior_builder_debt_before_new_work_and_does_not_pay_as_service() {
    let mut w = opening(5, 5);
    w.budget_data.employment.push(serde_json::from_value(json!({"employment_id":"builder-e","contract_id":"build-c","pop_id":"p","fte":2,"workers_count":2,"time_fraction":1})).unwrap());
    let budget = execute_budget_cycle(
        &mut w,
        &kingmaker_core::budget::BudgetRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
        },
    )
    .unwrap();
    assert!(!budget
        .operations
        .iter()
        .any(|op| op.kind == OperationKind::Payroll));
    assert_eq!(money(&w, "p", "personal_savings"), 5);
    execute_automatic_investments(&mut w, &request()).unwrap();
    let receipt = execute_construction_payroll(&mut w, &request()).unwrap();
    let op = receipt
        .operations
        .iter()
        .find(|op| op.kind == OperationKind::Payroll)
        .unwrap();
    assert_eq!(op.meta["actual_fte"], json!(0.));
    assert_eq!(money(&w, "p", "personal_savings"), 5);
}
#[test]
fn stale_project_stock_tax_or_history_rejects_before_builder_payment() {
    for case in 0..4 {
        let mut w = fixture(100);
        let prepared = prepare_construction_payroll(&w, &request()).unwrap();
        match case {
            0 => w.stockpiles[0].quantity += 1.,
            1 => {
                w.production_data.policies.taxes.insert("s".into(), 0.2);
            }
            2 => w.production_data.construction_projects[0].assignments[0].workers_count = 1.,
            _ => w.construction_data.phase_receipts.clear(),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(apply_construction_payroll(&mut w, prepared).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn builder_salary_survives_mini_load_and_rejects_lost_or_changed_required_operations() {
    let mut w = fixture(100);
    let receipt = execute_construction_payroll(&mut w, &request()).unwrap();
    let mut loaded: WorldState = serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    loaded.rebuild_account_index().unwrap();
    assert_eq!(
        execute_construction_payroll(&mut loaded, &request()).unwrap(),
        receipt
    );
    for case in 0..3 {
        let mut w = loaded.clone();
        match case {
            0 => w
                .production_data
                .committed_operations
                .retain(|op| op.value["type"] != json!("payroll")),
            1 => w
                .construction_data
                .phase_receipts
                .retain(|p| p.stage != kingmaker_core::construction::ConstructionStage::Payroll),
            _ => w
                .construction_data
                .phase_receipts
                .last_mut()
                .unwrap()
                .operation_evidence
                .clear(),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_construction_payroll(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn late_invalid_currency_or_tax_cannot_publish_partial_builder_salary() {
    for case in 0..2 {
        let mut w = fixture(100);
        match case {
            0 => w.pops[0].currency_id = Some("FOREIGN".into()),
            _ => {
                w.production_data.policies.taxes.insert("s".into(), 1.1);
            }
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_construction_payroll(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
