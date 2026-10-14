#[path = "common/budget.rs"]
mod common;
#[path = "common/market.rs"]
mod market_fixture;
use common::*;
use kingmaker_core::{production::Extra, *};
use kingmaker_sim::budget_cycle::{apply_budget_cycle, execute_budget_cycle, prepare_budget_cycle};
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn omitted_and_partial_policies_preserve_authored_taxes_and_wages() {
    for partial in [false, true] {
        let mut w = world(4000);
        adv(&mut w);
        w.production_data.policies.wages.insert("s".into(), 0.5);
        let mut r = request(&w);
        r.policies = Default::default();
        if partial {
            r.policies.taxes.insert("unrelated".into(), 0.4);
            r.policies.wages.insert("unrelated".into(), 0.1);
        }
        let receipt = execute_budget_cycle(&mut w, &r).unwrap();
        assert_eq!(cash(&w, "a", "personal_savings"), 1800);
        let teacher = receipt
            .operations
            .iter()
            .find(|o| o.meta.get("pop_id") == Some(&json!("teacher")))
            .unwrap();
        assert_eq!(teacher.meta["gross_minor"], json!("500"));
        assert_eq!(teacher.meta["tax_minor"], json!("50"));
    }
}
#[test]
fn orphan_zero_money_payroll_cannot_erase_unpaid_claims() {
    let mut prior = world(0);
    let r = request(&prior);
    let receipt = execute_budget_cycle(&mut prior, &r).unwrap();
    let op = receipt
        .operations
        .iter()
        .find(|o| o.meta.get("pop_id") == Some(&json!("teacher")))
        .unwrap();
    let mut w = world(0);
    w.production_data
        .committed_operations
        .push(kingmaker_core::operations::CommittedOperation {
            event_id: op.event_id.clone(),
            value: serde_json::to_value(op).unwrap(),
        });
    let before = w.clone();
    assert!(execute_budget_cycle(&mut w, &r).is_err());
    assert_eq!(w, before);
}
#[test]
fn intervening_empty_market_blocks_prepared_budget_publication() {
    let mut w = market_fixture::market_world();
    let r = request(&w);
    let p = prepare_budget_cycle(&w, &r).unwrap();
    let mut mr = kingmaker_core::market::MarketRequest {
        expected_revision: 20,
        period: r.period.clone(),
        stage: kingmaker_core::market::MarketStage::Maintenance,
        profile: Default::default(),
        priorities: None,
    };
    mr.period = r.period.clone();
    let mut market_copy = w.clone();
    let m = kingmaker_sim::market_cycle::execute_market_phase(&mut market_copy, &mr).unwrap();
    assert_eq!(m.operations.len(), 1);
    assert_eq!(
        m.operations[0].kind,
        kingmaker_core::operations::OperationKind::MarketPhaseBoundary
    );
    assert!(
        m.operations[0].debits.is_empty()
            && m.operations[0].credits.is_empty()
            && m.operations[0].stock_changes.is_empty()
    );
    w.market_data.phase_receipts.push(m);
    let before = w.clone();
    assert!(apply_budget_cycle(&mut w, p).is_err());
    assert_eq!(w, before);
}

#[test]
fn replay_returns_same_receipt_and_missing_or_conflicting_history_is_an_error() {
    let mut w = world(1000);
    adv(&mut w);
    let r = request(&w);
    let first = execute_budget_cycle(&mut w, &r).unwrap();
    let after = w.clone();
    assert_eq!(execute_budget_cycle(&mut w, &r).unwrap(), first);
    assert_eq!(w, after);
    w.production_data.committed_operations.pop();
    let missing = w.clone();
    assert!(execute_budget_cycle(&mut w, &r).is_err());
    assert_eq!(w, missing);
}
#[test]
fn prepared_read_set_rejects_changed_rate_employment_class_and_foreign_account_atomically() {
    for kind in 0..4 {
        let mut w = world(1000);
        adv(&mut w);
        let r = request(&w);
        let prepared = prepare_budget_cycle(&w, &r).unwrap();
        match kind {
            0 => {
                w.production_data.contracts[0]
                    .weekly_wage_rate
                    .as_mut()
                    .unwrap()
                    .numerator = "999".into()
            }
            1 => w.budget_data.employment[0].fte = 0.,
            2 => w.pops[2].pop_type_id = "aristocrats".into(),
            _ => w.accounts[0].balance = Money(0),
        };
        let before = w.clone();
        assert!(apply_budget_cycle(&mut w, prepared).is_err());
        assert_eq!(w, before);
    }
}
#[test]
fn cycle_matches_real_js_and_explains_only_authorized_money_creation() {
    for (pct, cash0, with_adv) in [(0., 500, false), (50., 1000, false), (100., 1000, true)] {
        let mut w = world(cash0);
        funding(&mut w, pct, 100.);
        if with_adv {
            adv(&mut w);
        }
        let r = request(&w);
        let extra = Extra::new();
        let pops:Vec<_>=w.pops.iter().map(|p|json!({"pop_id":p.external_id,"province_id":p.province_id.0,"state_id":p.state_id,"currency_id":p.currency_id,"pop_type_id":p.pop_type_id,"size":p.size,"workforce":p.workforce})).collect();
        let accounts:Vec<_>=w.accounts.iter().map(|a|json!({"owner_id":a.owner_id,"currency_id":a.currency_id,"purpose":a.purpose,"amount_minor":a.balance})).collect();
        let snapshot = json!({"campaign":{"campaign_id":"budget","date":{"year":306,"month":1,"day":1}},"accounts":accounts,"stocks":[],"reservations":[],"obligations":[],"appliedEventIds":[],"registry":{"pops":pops,"employment":w.budget_data.employment,"jobs":w.budget_data.jobs,"contracts":w.production_data.contracts,"policies":r.policies,"institutions":[],"resourceOperations":[],"factoryEnterprises":[],"artisanEnterprises":[],"constructionProjects":[],"maintenanceAgreements":[],"adventurerIncomeRates":if with_adv{serde_json::to_value(&w.budget_data.adventurer_income_rates).unwrap()}else{Value::Null},"foundationBudgetPolicy":null,"unused":extra}});
        let mut child = Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/tests/budget-oracle.cjs"
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(json!({"snapshot":snapshot,"action":"opening","options":{"period":"w","policies":r.policies}}).to_string().as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let oracle: Value = serde_json::from_slice(&output.stdout).unwrap();
        let before = w.clone();
        let receipt = execute_budget_cycle(&mut w, &r).unwrap();
        let mut native: Vec<_> = w
            .accounts
            .iter()
            .map(|a| json!([a.owner_id, a.currency_id, a.purpose, a.balance]))
            .collect();
        let mut js: Vec<_> = oracle["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| {
                json!([
                    a["owner_id"],
                    a["currency_id"],
                    a["purpose"],
                    a["amount_minor"]
                ])
            })
            .collect();
        native.sort_by_key(Value::to_string);
        js.sort_by_key(Value::to_string);
        assert_eq!(native, js);
        let net = w
            .accounts
            .iter()
            .map(|a| i128::from(a.balance.0))
            .sum::<i128>()
            - before
                .accounts
                .iter()
                .map(|a| i128::from(a.balance.0))
                .sum::<i128>();
        assert_eq!(net, if with_adv { 2000 } else { 0 });
        assert_eq!(w.stockpiles, before.stockpiles);
        assert_eq!(
            w.pops
                .iter()
                .map(|p| (p.external_id.clone(), p.size, p.workforce, p.employed))
                .collect::<Vec<_>>(),
            before
                .pops
                .iter()
                .map(|p| (p.external_id.clone(), p.size, p.workforce, p.employed))
                .collect::<Vec<_>>()
        );
        let boundaries=receipt.operations.iter().filter(|op|op.kind==kingmaker_core::operations::OperationKind::BudgetPhaseBoundary).collect::<Vec<_>>();
        assert_eq!(boundaries.len(),2);
        assert!(boundaries.iter().all(|op|op.debits.is_empty() && op.credits.is_empty() && op.stock_changes.is_empty()));
        let financial=receipt.operations.iter().filter(|op|op.kind!=kingmaker_core::operations::OperationKind::BudgetPhaseBoundary).collect::<Vec<_>>();
        assert_eq!(financial.len(),oracle["operations"].as_array().unwrap().len());
        for op in financial {
            let event = op.event_id.json();
            let j = oracle["operations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|o| o["event_id"] == event)
                .unwrap();
            for k in [
                "type",
                "gross_minor",
                "paid_gross_minor",
                "tax_minor",
                "net_minor",
                "actual_fte",
                "budget_state_id",
                "budget_sector",
                "source",
            ] {
                if let Some(value) = serde_json::to_value(op).unwrap().get(k) {
                    if k == "actual_fte" {
                        assert_eq!(value.as_f64(), j[k].as_f64(), "field {k} of {event}");
                    } else {
                        assert_eq!(value, &j[k], "field {k} of {event}");
                    }
                }
            }
            let mut native_op = serde_json::to_value(op).unwrap();
            let mut js_op = j.clone();
            for value in [&mut native_op, &mut js_op] {
                let object = value.as_object_mut().unwrap();
                // Rust receipt context; JS carries the week context outside the DTO.
                object.remove("period");
                object.remove("idle_guarantee_minor");
                for field in ["debits", "credits", "stock_changes"] {
                    if !object.contains_key(field) {
                        object.insert(field.into(), json!([]));
                    }
                }
                if !object.contains_key("obligation") {
                    object.insert("obligation".into(), Value::Null);
                }
                if let Some(debt) = object.get_mut("obligation").and_then(Value::as_object_mut) {
                    for field in ["payee_purpose", "category"] {
                        if !debt.contains_key(field) {
                            debt.insert(field.into(), Value::Null);
                        }
                    }
                }
                for field in ["planned_fte", "actual_fte", "tax_rate"] {
                    if let Some(v) = object.get_mut(field) {
                        *v = json!(v.as_f64().unwrap());
                    }
                }
            }
            assert_eq!(native_op, js_op, "full operation {event}");
        }
    }
}
