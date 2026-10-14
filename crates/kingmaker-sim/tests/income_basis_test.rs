#[path = "common/labor.rs"]
mod common;
use kingmaker_core::income::IncomeRequest;
use kingmaker_sim::income_basis::{capture_income_opening, validate_income_basis};
use serde_json::json;

fn request() -> IncomeRequest {
    IncomeRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    }
}

#[test]
fn income_basis_pins_native_opening_stock_metadata_without_freezing_current_world() {
    let mut w = common::world(100000);
    let before = serde_json::to_value(&w).unwrap();
    let basis = capture_income_opening(&w, &request()).unwrap();
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let frozen = serde_json::to_value(&basis).unwrap();
    w.stockpiles[0].owner_id = "new-owner".into();
    w.stockpiles[0].inventory_origin = Some("produced".into());
    assert_eq!(serde_json::to_value(&basis).unwrap(), frozen);
    assert_ne!(frozen["stocks"][0]["owner_id"], json!("new-owner"));
    validate_income_basis(&w, &request(), &basis).unwrap();
}

#[test]
fn income_basis_rejects_cross_campaign_revision_period_and_late_capture() {
    let mut w = common::world(100000);
    let basis = capture_income_opening(&w, &request()).unwrap();
    let mut r = request();
    r.period = "other".into();
    assert!(validate_income_basis(&w, &r, &basis).is_err());
    w.campaign.campaign_id = "other".into();
    assert!(validate_income_basis(&w, &request(), &basis).is_err());
    let mut w = common::world(100000);
    w.campaign.revision += 1;
    assert!(capture_income_opening(&w, &request()).is_err());
    let mut w = common::world(100000);
    let br = kingmaker_core::budget::BudgetRequest {
        expected_revision: 20,
        period: "w".into(),
        policies: Default::default(),
    };
    kingmaker_sim::budget_cycle::execute_budget_cycle(&mut w, &br).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert!(capture_income_opening(&w, &request()).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
