#[path = "common/budget.rs"]
mod budget;
#[path = "common/labor.rs"]
mod labor;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::{labor::LaborRequest, market::*, production::*, *};
use kingmaker_sim::{
    budget_cycle::execute_budget_cycle,
    labor_history_cycle::{apply_labor_history, execute_labor_history, prepare_labor_history},
    market_cycle::execute_market_phase,
    production::execute_production_cycle,
};
use serde_json::json;
#[test]
fn closed_history_and_payroll_evidence_survive_native_roundtrip() {
    let mut w = fixture(4000);
    close(&mut w);
    let receipt = execute_labor_history(&mut w, &request()).unwrap();
    let value = serde_json::to_value(&w).unwrap();
    let mut restored: WorldState = serde_json::from_value(value.clone()).unwrap();
    restored.rebuild_account_index().unwrap();
    assert_eq!(
        execute_labor_history(&mut restored, &request()).unwrap(),
        receipt
    );
    assert_eq!(serde_json::to_value(&restored).unwrap(), value);
}
fn fixture(cash: i64) -> WorldState {
    let mut w = budget::world(cash);
    let c = labor::world(0);
    w.production_data.catalog = Some(ProductionCatalog {
        version: "empty".into(),
        goods: vec![],
        methods: vec![],
        extra: Default::default(),
    });
    w.market_data = c.market_data;
    w.market_data
        .needs
        .as_mut()
        .unwrap()
        .profile_refs
        .truncate(1);
    w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = "teacher".into();
    let mut r = w.market_data.needs.as_ref().unwrap().profile_refs[0].clone();
    r.pop_id = "medic".into();
    w.market_data.needs.as_mut().unwrap().profile_refs.push(r);
    let price = w
        .market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .remove("COIN")
        .unwrap();
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .insert("X".into(), price);
    w.labor_data.pop_mobility = Some(json!({"unpaid_weeks":{"teacher":2},"custom":"keep"}));
    w
}
fn request() -> LaborRequest {
    LaborRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    }
}
#[test]
fn altered_production_payroll_cannot_reset_unpaid_history() {
    let mut w = production::world();
    let data = labor::world(0);
    w.market_data = data.market_data;
    w.market_data
        .needs
        .as_mut()
        .unwrap()
        .profile_refs
        .truncate(1);
    w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = "p".into();
    w.labor_data.pop_mobility = Some(json!({"unpaid_weeks":{"e":3}}));
    close(&mut w);
    let op = w
        .production_data
        .committed_operations
        .iter_mut()
        .find(|e| e.value["type"] == json!("payroll"))
        .unwrap();
    op.value["paid_gross_minor"] = json!("0");
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_labor_history(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn changing_required_production_payroll_kind_cannot_hide_missing_payment_evidence() {
    let mut w = production::world();
    w.market_data = labor::world(0).market_data;
    w.market_data
        .needs
        .as_mut()
        .unwrap()
        .profile_refs
        .truncate(1);
    w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = "p".into();
    w.labor_data.pop_mobility = Some(json!({"unpaid_weeks":{"e":3}}));
    close(&mut w);
    let op = w
        .production_data
        .committed_operations
        .iter_mut()
        .find(|e| e.value["type"] == json!("payroll"))
        .unwrap();
    op.value["type"] = json!("blocked");
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_labor_history(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn removing_required_payroll_event_from_production_list_rejects_without_writes() {
    let mut w = production::world();
    w.market_data = labor::world(0).market_data;
    w.market_data
        .needs
        .as_mut()
        .unwrap()
        .profile_refs
        .truncate(1);
    w.market_data.needs.as_mut().unwrap().profile_refs[0].pop_id = "p".into();
    close(&mut w);
    let paid = w.production_data.phase_receipts[0].payroll[0]
        .event_id
        .clone();
    w.production_data.phase_receipts[0]
        .event_ids
        .retain(|id| id != &paid);
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_labor_history(&mut w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
fn close(w: &mut WorldState) {
    execute_budget_cycle(w, &budget::request(w)).unwrap();
    execute_production_cycle(
        w,
        &ProductionRequest {
            expected_revision: 20,
            period: "w".into(),
            policies: Default::default(),
            profile: Default::default(),
            maintenance_receipts: vec![],
        },
    )
    .unwrap();
    execute_market_phase(
        w,
        &MarketRequest {
            expected_revision: 20,
            period: "w".into(),
            stage: MarketStage::ClosingMarket,
            profile: Default::default(),
            priorities: None,
        },
    )
    .unwrap();
}
#[test]
fn committed_payroll_only_sets_history_and_preserves_money_date_and_replay() {
    let mut w = fixture(4000);
    close(&mut w);
    let money = w.accounts.clone();
    let date = w.campaign.clone();
    let r = execute_labor_history(&mut w, &request()).unwrap();
    assert_eq!(
        w.labor_data.pop_mobility.as_ref().unwrap()["unpaid_weeks"]["teacher"],
        json!(0)
    );
    assert_eq!(
        w.labor_data.pop_mobility.as_ref().unwrap()["custom"],
        json!("keep")
    );
    assert_eq!(w.accounts, money);
    assert_eq!(w.campaign, date);
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_labor_history(&mut w, &request()).unwrap(), r);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn missing_phase_or_tampered_required_payroll_rejects_without_history_mutation() {
    for case in [0, 1] {
        let mut w = fixture(4000);
        if case == 1 {
            close(&mut w);
            let op = w
                .production_data
                .committed_operations
                .iter_mut()
                .find(|e| e.value["type"] == json!("payroll"))
                .unwrap();
            op.value["paid_gross_minor"] = json!("99999");
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_labor_history(&mut w, &request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn zero_pay_and_old_period_history_are_handled_as_real_facts() {
    let mut w = fixture(0);
    close(&mut w);
    w.production_data.committed_operations.push(kingmaker_core::operations::CommittedOperation{event_id:"old-record".into(),value:json!({"event_id":"old-record","type":"payroll","period":"older","paid_gross_minor":"999999","employment_id":"teacher"})});
    execute_labor_history(&mut w, &request()).unwrap();
    assert_eq!(
        w.labor_data.pop_mobility.as_ref().unwrap()["unpaid_weeks"]["teacher"],
        json!(3)
    );
}
#[test]
fn prepared_history_rejects_changed_roster_or_actual_payment() {
    let mut w = fixture(4000);
    close(&mut w);
    let p = prepare_labor_history(&w, &request()).unwrap();
    w.pops[0].size += 1;
    let before = serde_json::to_value(&w).unwrap();
    assert!(apply_labor_history(&mut w, p).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
