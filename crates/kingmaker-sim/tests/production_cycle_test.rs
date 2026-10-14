#[path = "common/production.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::production::*;
#[test]
fn production_replay_requires_complete_unchanged_operation_evidence() {
    for case in 0..3 {
        let mut w = common::world();
        let request = common::request();
        let receipt = execute_production_cycle(&mut w, &request).unwrap();
        match case {
            0 => w.production_data.committed_operations.clear(),
            1 => {
                w.production_data
                    .committed_operations
                    .iter_mut()
                    .find(|op| op.value["type"] == serde_json::json!("production"))
                    .unwrap()
                    .value["output_quantity"] = serde_json::json!(999)
            }
            _ => w.production_data.phase_receipts.push(receipt),
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_production_cycle(&mut w, &request).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn actual_cycle_consumes_owned_recipe_pays_tax_stores_output_and_replays() {
    let mut w = common::world();
    let request = common::request();
    let receipt = execute_production_cycle(&mut w, &request).unwrap();
    assert_eq!(receipt.outputs[0].quantity, 1.0);
    assert_eq!(w.stockpiles[0].quantity, 0.0);
    assert_eq!(w.stockpiles[1].quantity, 0.0);
    assert_eq!(w.stockpiles[2].quantity, 1.0);
    assert_eq!(w.stockpiles[2].owner_id, ExternalId::from("o"));
    assert_eq!(w.production_data.factories[0].tenure_days, Some(7.0));
    assert_eq!(w.production_data.storage_reservations[0].status, "consumed");
    let before = w.clone();
    assert_eq!(execute_production_cycle(&mut w, &request).unwrap(), receipt);
    assert_eq!(w, before);
    let mut different = request.clone();
    different.policies.wages.insert("o".into(), 0.0);
    assert!(execute_production_cycle(&mut w, &different).is_err());
    assert_eq!(w, before);
}
#[test]
fn missing_currency_at_last_operation_rolls_back_wages_and_storage() {
    let mut w = common::world();
    w.production_data.factories[0].base.payment_purpose = Some("unknown".into());
    let before = w.clone();
    assert!(execute_production_cycle(&mut w, &common::request()).is_err());
    assert_eq!(w, before);
}
#[test]
fn author_changes_and_assignments_apply_next_period_without_counter_patch() {
    let mut w = common::world();
    execute_production_cycle(&mut w, &common::request()).unwrap();
    w.stockpiles[0].quantity = 2.2;
    w.stockpiles[1].quantity = 0.8;
    w.accounts[0].balance = Money(20);
    w.production_data.factories[0].equipment_capacity_weekly = Some(0.5);
    let mut request = common::request();
    request.period = "w2".into();
    let receipt = execute_production_cycle(&mut w, &request).unwrap();
    assert_eq!(receipt.outputs[0].quantity, 0.5);
    assert_eq!(w.enterprises[0].employees, Some(1.0));
    assert_eq!(w.production_data.factories[0].tenure_days, Some(10.5));
}
#[test]
fn shared_inputs_arent_free_and_foreign_food_stocks_remain_owned() {
    let mut w = common::world();
    w.stockpiles[0].owner_id = "other".into();
    w.stockpiles[1].purpose = "food_reserve".into();
    let receipt = execute_production_cycle(&mut w, &common::request()).unwrap();
    assert_eq!(receipt.outputs[0].quantity, 0.0);
    assert_eq!(w.stockpiles[0].quantity, 1.1);
    assert_eq!(w.stockpiles[1].quantity, 0.4);
    assert_eq!(w.production_data.factories[0].tenure_days, Some(0.0));
    assert_eq!(receipt.payroll[0].gross_minor, Money(2));
}
