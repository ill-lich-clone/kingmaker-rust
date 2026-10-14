use kingmaker_core::*;
use kingmaker_sim::construction_rights::{complete_project_ownership, plan_project_refunds};
use serde_json::json;
#[path = "common/production.rs"]
mod common;
fn fixture() -> (WorldState, serde_json::Value) {
    let mut w = common::world();
    w.accounts
        .push(common::account(3, "project:p", "construction_escrow", 6000));
    w.accounts.push(common::account(
        4,
        "project:p",
        "factory_startup_reserve",
        4000,
    ));
    w.rebuild_account_index().unwrap();
    w.mobility_data.project_investments = vec![
        json!({"project_id":"p","investor_id":"a","owner_id":"a","source_account_purpose":"personal_savings","currency_id":"COIN","amount_minor":"3000"}),
        json!({"project_id":"p","investor_id":"b","owner_id":"b","source_account_purpose":"personal_savings","currency_id":"COIN","amount_minor":"7000"}),
    ];
    (
        w,
        json!({"id":"p","kind":"factory","owner_id":"project:p","province_id":1,"region_id":"r","method_id":"factory:steel","funded_minor":"10000","terms":{"budget_minor":"10000","currency_id":"COIN"},"construction":{"source":"fixture"}}),
    )
}
#[test]
fn actual_investor_capital_and_net_refunds_determine_rational_asset_rights() {
    let (w, mut p) = fixture();
    let ownership = complete_project_ownership(&w, &p).unwrap();
    assert_eq!(
        ownership["shares"],
        json!([{"investor_id":"a","owner_id":"a","numerator":"3","denominator":"10"},{"investor_id":"b","owner_id":"b","numerator":"7","denominator":"10"}])
    );
    assert_eq!(ownership["asset"]["owner_id"], json!("enterprise:p"));
    p["refunded_minor"] = json!("5000");
    p["investor_refunds"] = json!([{"investor_id":"a","owner_id":"a","amount_minor":"1500"},{"investor_id":"b","owner_id":"b","amount_minor":"3500"}]);
    assert_eq!(
        complete_project_ownership(&w, &p).unwrap()["shares"],
        ownership["shares"]
    );
}
#[test]
fn refunds_debit_real_project_accounts_and_preserve_each_original_purpose() {
    let (w, p) = fixture();
    let before = serde_json::to_value(&w).unwrap();
    let refund = plan_project_refunds(&w, &p, "cancel").unwrap();
    assert_eq!(refund.operations.len(), 2);
    assert_eq!(
        refund
            .refunds
            .iter()
            .filter(|r| r["investor_id"] == json!("a"))
            .map(|r| r["amount_minor"].as_str().unwrap().parse::<i64>().unwrap())
            .sum::<i64>(),
        3000
    );
    assert_eq!(
        refund
            .refunds
            .iter()
            .filter(|r| r["investor_id"] == json!("b"))
            .map(|r| r["amount_minor"].as_str().unwrap().parse::<i64>().unwrap())
            .sum::<i64>(),
        7000
    );
    for op in refund.operations {
        assert!(op.credits.iter().all(|c| c.purpose == "personal_savings"));
        assert_eq!(
            op.debits.iter().map(|d| d.amount_minor.0).sum::<i64>(),
            op.credits.iter().map(|c| c.amount_minor.0).sum::<i64>()
        );
    }
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn wrong_currency_missing_rights_purpose_conflict_and_overrefund_are_rejected() {
    for case in 0..4 {
        let (mut w, mut p) = fixture();
        match case {
            0 => w.mobility_data.project_investments[0]["currency_id"] = json!("OTHER"),
            1 => w.mobility_data.project_investments.clear(),
            2 => {
                let mut row = w.mobility_data.project_investments[0].clone();
                row["source_account_purpose"] = json!("other");
                w.mobility_data.project_investments.push(row);
            }
            _ => {
                p["refunded_minor"] = json!("4000");
                p["investor_refunds"] =
                    json!([{"investor_id":"a","owner_id":"a","amount_minor":"4000"}]);
            }
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(complete_project_ownership(&w, &p).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
