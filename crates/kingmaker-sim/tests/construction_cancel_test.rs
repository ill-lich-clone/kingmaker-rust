#[path = "common/construction.rs"]
mod common;
#[path = "common/income.rs"]
mod income_common;
use kingmaker_core::{construction::ConstructionCancellationRequest, *};
use kingmaker_sim::construction_cancellation::{
    apply_cancellation, execute_cancellation, prepare_cancellation,
};
use serde_json::json;
fn request() -> ConstructionCancellationRequest {
    ConstructionCancellationRequest {
        expected_revision: 20,
        event_id: "cancel-test".into(),
        project_id: "private-project:offer".into(),
        reason: "author decision".into(),
    }
}
#[test]
fn cancellation_returns_only_unspent_cash_and_retains_actual_goods_and_cargo_claims() {
    for progressed in [false, true] {
        let mut w = common::ready(30);
        w.mobility_data.shipments.push(json!({"id":"cargo","buyer_id":"project:private-project:offer","goods_id":"iron","remaining_quantity":2,"to_province_id":1,"currency_id":"COIN","unknown_author_field":true}));
        if progressed {
            kingmaker_sim::construction_cycle::execute_construction(&mut w, &common::request())
                .unwrap();
        }
        let goods = w.stockpiles.clone();
        let cargo = w.mobility_data.shipments.clone();
        let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
        let receipt = execute_cancellation(&mut w, &request()).unwrap();
        assert!(receipt.invariants.all_required_passed());
        assert_eq!(
            w.production_data.construction_projects[0].status,
            "cancelled"
        );
        assert_eq!(common::cash(&w, "a", "personal_savings"), 2997);
        assert_eq!(common::cash(&w, "b", "personal_savings"), 6993);
        assert_eq!(w.stockpiles, goods);
        assert_eq!(w.mobility_data.shipments, cargo);
        assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
        let p = serde_json::to_value(&w.production_data.construction_projects[0]).unwrap();
        assert_eq!(
            p["remaining_cargo_claims"][0]["shipment_id"],
            json!("cargo")
        );
        assert_eq!(
            p["remaining_cargo_claims"][0]["shares"][0]["numerator"],
            json!("3")
        );
        assert_eq!(
            p["remaining_goods_claims"].as_array().unwrap().len(),
            usize::from(!progressed)
        );
        let before = serde_json::to_value(&w).unwrap();
        assert_eq!(execute_cancellation(&mut w, &request()).unwrap(), receipt);
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
        let mut loaded: WorldState = serde_json::from_value(before).unwrap();
        loaded.rebuild_account_index().unwrap();
        assert_eq!(
            execute_cancellation(&mut loaded, &request()).unwrap(),
            receipt
        );
        loaded.campaign.revision += 1;
        loaded.campaign.day += 7;
        assert_eq!(
            execute_cancellation(&mut loaded, &request()).unwrap(),
            receipt
        );
    }
}
#[test]
fn unfunded_proposal_can_be_cancelled_and_completed_factory_cannot() {
    let mut w = common::opening(7);
    w.production_data.construction_projects.push(serde_json::from_value(json!({"id":"private-project:offer","status":"awaiting_funding","owner_id":"project:private-project:offer","kind":"factory","province_id":1,"terms":{"currency_id":"COIN","budget_minor":"10000"},"funded_minor":"0","assignments":[]})).unwrap());
    execute_cancellation(&mut w, &request()).unwrap();
    let mut w = common::ready(7);
    kingmaker_sim::construction_cycle::execute_construction(&mut w, &common::request()).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_cancellation(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn stale_or_illegal_cash_and_conflicting_or_lost_history_rolls_back() {
    for case in 0..4 {
        let mut w = common::ready(30);
        let prepared = prepare_cancellation(&w, &request()).unwrap();
        match case {
            0 => w.accounts[0].balance = Money(1),
            1 => w.stockpiles[0].quantity += 1.,
            2 => w.mobility_data.project_investments[0]["owner_id"] = json!("other"),
            _ => w.production_data.committed_operations.clear(),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(apply_cancellation(&mut w, prepared).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
    let mut w = common::ready(30);
    execute_cancellation(&mut w, &request()).unwrap();
    let mut wrong = request();
    wrong.reason = "different".into();
    assert!(execute_cancellation(&mut w, &wrong).is_err());
    w.production_data
        .committed_operations
        .retain(|o| o.event_id != ExternalId::from("cancel-test"));
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_cancellation(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn excess_cash_missing_rights_wrong_currency_and_refund_purpose_conflict_reject() {
    for case in 0..5 {
        let mut w = common::ready(30);
        match case {
            0 => {
                w.accounts
                    .iter_mut()
                    .find(|a| a.purpose == "construction_escrow")
                    .unwrap()
                    .balance = Money(10001)
            }
            1 => w.mobility_data.project_investments.clear(),
            2 => w.mobility_data.project_investments[0]["currency_id"] = json!("other"),
            3 => {
                let mut i = w.mobility_data.project_investments[0].clone();
                i["source_account_purpose"] = json!("other");
                w.mobility_data.project_investments.push(i);
            }
            _ => {
                w.accounts
                    .iter_mut()
                    .find(|a| a.purpose == "factory_startup_reserve")
                    .unwrap()
                    .balance = Money(5000)
            }
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_cancellation(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn partial_project_refunds_round_original_purposes_and_match_source() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut w = common::opening(30);
    w.accounts[2].balance = Money(1);
    w.accounts[3].balance = Money(2);
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
    assert_eq!(
        w.production_data.construction_projects[0].extra["funded_minor"],
        json!("3")
    );
    let mut snapshot = income_common::snapshot(&w);
    snapshot["registry"]["constructionProjects"] = json!(w.production_data.construction_projects);
    snapshot["registry"]["projectInvestments"] = json!(w.mobility_data.project_investments);
    snapshot["registry"]["shipments"] = json!(w.mobility_data.shipments);
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
    child.stdin.take().unwrap().write_all(json!({"action":"cancel","snapshot":snapshot,"request":{"eventId":"cancel-test","projectId":"private-project:offer","reason":"author decision"}}).to_string().as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let expected: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let actual = execute_cancellation(&mut w, &request()).unwrap();
    let operations: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(expected["operations"].clone()).unwrap();
    assert_eq!(
        &actual.operations[..actual.operations.len() - 1],
        operations
    );
    assert_eq!(common::cash(&w, "a", "personal_savings"), 1);
    assert_eq!(common::cash(&w, "b", "personal_savings"), 2);
    let mut projects: Vec<kingmaker_core::production::ConstructionProjectLabor> =
        serde_json::from_value(expected["registry"]["constructionProjects"].clone()).unwrap();
    for p in &mut projects {
        for claim in p
            .extra
            .get_mut("remaining_goods_claims")
            .unwrap()
            .as_array_mut()
            .unwrap()
        {
            claim["amount"] = json!(claim["amount"].as_f64().unwrap());
        }
    }
    assert_eq!(w.production_data.construction_projects, projects);
}
#[test]
fn receipt_registry_input_history_and_claim_tampering_are_rejected() {
    for case in 0..4 {
        let mut w = common::ready(30);
        execute_cancellation(&mut w, &request()).unwrap();
        let index = w
            .construction_data
            .phase_receipts
            .iter()
            .position(|p| p.stage == kingmaker_core::construction::ConstructionStage::Cancellation)
            .unwrap();
        match case {
            0 => {
                w.construction_data.phase_receipts[index].registry_changes["project"]["owner_id"] =
                    json!("other")
            }
            1 => w.construction_data.phase_receipts[index]
                .operation_evidence
                .clear(),
            2 => w.production_data.construction_projects[0]
                .extra
                .insert("remaining_goods_claims".into(), json!([]))
                .map(|_| ())
                .unwrap_or(()),
            _ => w.production_data.committed_operations.retain(|o| {
                !o.value["type"]
                    .as_str()
                    .is_some_and(|s| s == "capitalist_investment")
            }),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_cancellation(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn rounding_across_two_project_accounts_cannot_refund_one_investor_twice() {
    let mut w = common::opening(7);
    w.accounts[2].balance = Money(1);
    w.accounts[3].balance = Money(1);
    let opportunity = &mut w.construction_data.investment_opportunities[0];
    opportunity["terms"]["budget_minor"] = json!("2");
    opportunity["terms"]["operating_reserve_minor"] = json!("1");
    w.production_data.catalog.as_mut().unwrap().methods[0]
        .extra
        .get_mut("construction")
        .unwrap()["cost_minor"] = json!("1");
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
    assert_eq!(
        common::cash(&w, "project:private-project:offer", "construction_escrow"),
        1
    );
    assert_eq!(
        common::cash(
            &w,
            "project:private-project:offer",
            "factory_startup_reserve"
        ),
        1
    );
    execute_cancellation(&mut w, &request()).unwrap();
    assert_eq!(common::cash(&w, "a", "personal_savings"), 1);
    assert_eq!(common::cash(&w, "b", "personal_savings"), 1);
}
