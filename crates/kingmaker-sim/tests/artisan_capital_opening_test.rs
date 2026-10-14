#[path = "common/native_week.rs"]
mod common;
#[path = "common/income.rs"]
mod source_snapshot;
use kingmaker_core::{income::IncomeRequest, *};
use kingmaker_sim::income_cycle::execute_artisan_opening;
use serde_json::json;
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn actual_whole_week_with_unavailable_recipe_keeps_it_unimplemented_without_output() {
    let mut w = world();
    w.production_data.catalog.as_mut().unwrap().methods[0].status = "unresolved".into();
    w.production_data.catalog.as_mut().unwrap().methods[0].weekly_productivity = None;
    w.production_data.artisans[0]
        .base
        .extra
        .insert("automatic_choice".into(), json!(false));
    let before_money: i128 = w.accounts.iter().map(|a| i128::from(a.balance.0)).sum();
    let request = common::request(&w);
    kingmaker_sim::native_week::execute_native_week(&mut w, &request, |_| {}).unwrap();
    assert_eq!(w.campaign.revision, 21);
    assert_eq!(
        w.accounts
            .iter()
            .map(|a| i128::from(a.balance.0))
            .sum::<i128>(),
        before_money
    );
    assert_eq!(
        w.production_data.catalog.as_ref().unwrap().methods[0].weekly_productivity,
        None
    );
    assert_eq!(
        w.production_data.catalog.as_ref().unwrap().methods[0].status,
        "unresolved"
    );
    assert!(w
        .production_data
        .phase_receipts
        .iter()
        .flat_map(|r| &r.outputs)
        .all(|o| o.quantity == 0.));
    assert_eq!(w.production_data.artisans.len(), 1);
}

#[test]
fn ready_missing_productivity_whole_week_failure_leaves_input_unchanged() {
    let mut w = world();
    w.production_data.catalog.as_mut().unwrap().methods[0].weekly_productivity = None;
    let before = serde_json::to_value(&w).unwrap();
    let request = common::request(&w);
    assert!(kingmaker_sim::native_week::execute_native_week(&mut w, &request, |_| {}).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}

fn run(w: &mut WorldState, r: &IncomeRequest) -> kingmaker_core::income::IncomePhaseReceipt {
    let mut p = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/income-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(json!({"action":"capital_opening","snapshot":source_snapshot::snapshot(w),"period":r.period,"policies":r.policies}).to_string().as_bytes()).unwrap();
    let out = p.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let source: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let source_operations: Vec<kingmaker_core::operations::EconomicOperation> =
        serde_json::from_value(source["operations"].clone()).unwrap();
    let mut source_enterprises: Vec<kingmaker_core::production::ArtisanEnterprise> =
        serde_json::from_value(source["enterprises"].clone()).unwrap();
    // JS Number renders integral physical quantities without `.0`; normalize
    // only that representation, preserving the actual value and every field.
    for e in &mut source_enterprises {
        if let Some(c) = &mut e.working_capital {
            c["feasible_output"] = json!(c["feasible_output"].as_f64().unwrap());
        }
    }
    let receipt = execute_artisan_opening(w, r).unwrap();
    assert_eq!(receipt.operations, source_operations);
    assert_eq!(w.production_data.artisans, source_enterprises);
    receipt
}

fn world() -> WorldState {
    let mut w = common::factory(false);
    w.pops[0].pop_type_id = "artisans".into();
    w.pops[0].pop_type = "artisans".into();
    let mut method = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    method.method_id = "artisan:steel".into();
    method.kind = "artisan".into();
    method.staffing = vec!["artisans".into()];
    w.production_data.catalog.as_mut().unwrap().methods = vec![method];
    w.production_data.factories.clear();
    w.production_data.artisans = serde_json::from_value(json!([{"id":"artisan-p","owner_id":"p","province_id":1,"method_id":"artisan:steel","payment_purpose":"operating_capital","facility_id":"wh","people_slots":1,"assignments":[{"pop_id":"p","workers_count":1,"time_fraction":1,"employment_id":"art-job"}],"income_policy":{"source":"author","cash_reserve_minor":"500","payout_fraction":1},"working_capital":{"currency_id":"COIN","personal_funded_minor":"0"}}])).unwrap();
    w.accounts.push(AccountBalance {
        id: AccountId(100),
        external_id: "personal-p".into(),
        owner_id: "p".into(),
        currency_id: "COIN".into(),
        purpose: "personal_savings".into(),
        balance: Money(1000),
    });
    w.rebuild_account_index().unwrap();
    w
}
fn request(w: &WorldState) -> IncomeRequest {
    IncomeRequest {
        expected_revision: w.campaign.revision,
        period: "capital-review".into(),
        policies: Default::default(),
    }
}
fn cash(w: &WorldState, owner: &str, purpose: &str) -> i64 {
    w.account_by_address(&AccountAddress {
        owner_id: owner.into(),
        currency_id: "COIN".into(),
        purpose: purpose.into(),
    })
    .map(|a| a.balance.0)
    .unwrap_or(0)
}
#[test]
fn opening_funds_real_artisan_reserve_once_and_preserves_total_cash() {
    let mut w = world();
    let r = request(&w);
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let receipt = run(&mut w, &r);
    assert_eq!(cash(&w, "p", "personal_savings"), 500);
    assert_eq!(cash(&w, "p", "operating_capital"), 500);
    assert_eq!(
        w.production_data.artisans[0]
            .working_capital
            .as_ref()
            .unwrap()["personal_funded_minor"],
        json!("500")
    );
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_artisan_opening(&mut w, &r).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn opening_combines_two_enterprises_without_reusing_the_owners_personal_cash() {
    let mut w = world();
    let mut second = w.production_data.artisans[0].clone();
    second.base.id = "artisan-p-2".into();
    w.production_data.artisans.push(second);
    w.accounts.last_mut().unwrap().balance = Money(700);
    let r = request(&w);
    run(&mut w, &r);
    assert_eq!(cash(&w, "p", "personal_savings"), 0);
    assert_eq!(cash(&w, "p", "operating_capital"), 700);
    for e in &w.production_data.artisans {
        assert_eq!(
            e.working_capital.as_ref().unwrap()["personal_funded_minor"],
            json!("350")
        );
    }
}

fn business(w: &mut WorldState, amount: i64, principal: &str, retained: &str, active: bool) {
    w.accounts.push(AccountBalance {
        id: AccountId(101),
        external_id: "business-p".into(),
        owner_id: "p".into(),
        currency_id: "COIN".into(),
        purpose: "operating_capital".into(),
        balance: Money(amount),
    });
    w.production_data.artisans[0]
        .working_capital
        .as_mut()
        .unwrap()["personal_funded_minor"] = json!(principal);
    w.production_data.artisans[0]
        .base
        .extra
        .insert("undistributed_profit_minor".into(), json!(retained));
    if !active {
        w.production_data.artisans[0].base.assignments.clear();
    }
    w.rebuild_account_index().unwrap();
}
#[test]
fn active_capital_release_is_limited_to_personally_funded_principal() {
    let mut w = world();
    business(&mut w, 1000, "200", "300", true);
    let r = request(&w);
    run(&mut w, &r);
    assert_eq!(cash(&w, "p", "personal_savings"), 1200);
    assert_eq!(cash(&w, "p", "operating_capital"), 800);
    assert_eq!(
        w.production_data.artisans[0]
            .working_capital
            .as_ref()
            .unwrap()["personal_funded_minor"],
        json!("0")
    );
    assert_eq!(
        w.production_data.artisans[0].base.extra["undistributed_profit_minor"],
        json!("300")
    );
}
#[test]
fn inactive_retained_profit_is_taxed_and_remaining_principal_released_once() {
    let mut w = world();
    business(&mut w, 1000, "200", "300", false);
    let mut r = request(&w);
    let state = w.pops[0].state_id.clone().unwrap();
    r.policies
        .taxes
        .insert(state.as_str().unwrap().to_owned(), 0.1);
    let total = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let receipt = run(&mut w, &r);
    assert_eq!(cash(&w, "p", "personal_savings"), 1470);
    assert_eq!(cash(&w, "p", "operating_capital"), 500);
    assert_eq!(
        cash(
            &w,
            &format!("treasury:{}", state.as_str().unwrap()),
            "operating_capital"
        ),
        30
    );
    assert_eq!(
        w.production_data.artisans[0].base.extra["undistributed_profit_minor"],
        json!("0")
    );
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), total);
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_artisan_opening(&mut w, &r).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}

#[test]
fn opening_can_finance_a_profitable_retooling_without_counting_its_cost_as_principal() {
    let mut w = world();
    w.production_data.catalog.as_mut().unwrap().methods[0].status = "planned".into();
    let mut next = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    next.method_id = "artisan:next".into();
    next.status = "ready".into();
    next.inputs.clear();
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .push(next);
    w.production_data.artisans[0]
        .base
        .extra
        .insert("automatic_choice".into(), json!(true));
    w.production_data.artisans[0].base.extra.insert("retooling".into(), json!({"source":"author","provider_id":"employer","currency_id":"COIN","purpose":"operating_capital","cost_minor":"100","duration_days":7}));
    let r = request(&w);
    let provider = cash(&w, "employer", "operating_capital");
    run(&mut w, &r);
    let e = &w.production_data.artisans[0];
    assert_eq!(e.base.method_id, ExternalId::from("artisan:steel"));
    assert_eq!(
        e.pending_retooling.as_ref().unwrap()["method_id"],
        json!("artisan:next")
    );
    assert_eq!(
        e.pending_retooling.as_ref().unwrap()["ready_date"],
        json!({"year":306,"month":1,"day":8})
    );
    assert_eq!(cash(&w, "employer", "operating_capital"), provider + 100);
    assert_eq!(cash(&w, "p", "personal_savings"), 400);
    assert_eq!(cash(&w, "p", "operating_capital"), 500);
    assert_eq!(
        e.working_capital.as_ref().unwrap()["personal_funded_minor"],
        json!("500")
    );
}

#[test]
fn invalid_inactive_property_tax_refuses_without_publishing_a_partial_payment() {
    let mut w = world();
    business(&mut w, 1000, "200", "300", false);
    let mut r = request(&w);
    r.policies.taxes.insert(
        w.pops[0]
            .state_id
            .as_ref()
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned(),
        1.1,
    );
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_artisan_opening(&mut w, &r).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}

#[test]
fn opening_preserves_the_selected_profile_and_rejects_public_capital_payloads() {
    use kingmaker_sim::{
        income_cycle::execute_artisan_opening_with_profile, operation_session::OperationSession,
    };
    let mut w = world();
    let r = request(&w);
    let profile = kingmaker_core::market::MarketProfile {
        food_norm_multiplier: Some(2.),
        extra: Default::default(),
    };
    let receipt = execute_artisan_opening_with_profile(&mut w, &r, &profile).unwrap();
    assert_eq!(receipt.opening_profile, Some(profile.clone()));
    assert_eq!(
        execute_artisan_opening_with_profile(&mut w, &r, &profile).unwrap(),
        receipt
    );
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_artisan_opening(&mut w, &r).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut original = world();
    for kind in ["artisan_capital_funding", "artisan_capital_release"] {
        let op = serde_json::from_value(json!({"event_id":"public-capital","type":kind,"debits":[{"owner_id":"p","currency_id":"COIN","purpose":"personal_savings","amount_minor":"100"}],"credits":[{"owner_id":"p","currency_id":"COIN","purpose":"operating_capital","amount_minor":"100"}]})).unwrap();
        let mut session = OperationSession::new(&original).unwrap();
        assert!(session.stage(&op).is_err());
        assert!(session.finish().is_err());
    }
    assert_eq!(cash(&original, "p", "personal_savings"), 1000);
    original.rebuild_account_index().unwrap();
}
