#[path = "common/income.rs"]
mod common;
#[path = "common/production.rs"]
mod production_common;
use kingmaker_core::income::StockContribution;
use kingmaker_core::*;
use kingmaker_sim::{
    income_basis::IncomeOpeningBasis,
    income_cycle::{
        apply_income_closing, execute_artisan_opening, execute_income_closing,
        prepare_income_closing,
    },
};
use serde_json::json;
fn fixture(artisan: bool) -> (WorldState, IncomeOpeningBasis) {
    let (w, basis, _) = common::fixture_configure(false, |w| {
        let mut ar = w.pops[0].clone();
        ar.id = PopId(9);
        ar.external_id = "ar".into();
        ar.pop_type_id = "aristocrats".into();
        ar.pop_type = "aristocrats".into();
        ar.workforce = 0.;
        ar.employed = 0.;
        w.pops.push(ar);
        w.mobility_data.production_beneficiaries = vec![
            json!({"owner_id":"o","currency_id":"COIN","source":"fixture ownership","beneficiaries":[{"owner_id":"ar","pop_id":"ar","purpose":"personal_savings","state_id":"s","numerator":"1","denominator":"1"}]}),
        ];
        w.production_data.policies.taxes.insert("s".into(), 0.1);
        if artisan {
            let mut q = w.pops[0].clone();
            q.id = PopId(8);
            q.external_id = "q".into();
            q.pop_type_id = "artisans".into();
            q.pop_type = "artisans".into();
            w.pops.push(q);
            let mut method = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
            method.kind = "artisan".into();
            method.method_id = "artisan:steel".into();
            method.staffing = vec!["artisans".into()];
            w.production_data
                .catalog
                .as_mut()
                .unwrap()
                .methods
                .push(method);
            w.production_data.artisans.push(serde_json::from_value(json!({"id":"a","owner_id":"b","province_id":1,"facility_id":"whb","method_id":"artisan:steel","people_slots":1,"assignments":[{"employment_id":"eq","pop_id":"q","workers_count":1,"time_fraction":1}],"income_policy":{"source":"fixture","cash_reserve_minor":"0","payout_fraction":1}})).unwrap());
            let mut facility = w.production_data.storage_facilities[0].clone();
            facility.id = "whb".into();
            facility.owner_id = "b".into();
            w.production_data.storage_facilities.push(facility);
            for (id, index) in [(30, 0), (31, 1)] {
                let mut stock = w.stockpiles[index].clone();
                stock.id = id;
                stock.external_id = format!("b:{id}").into();
                stock.owner_id = "b".into();
                w.stockpiles.push(stock);
            }
            w.accounts.push(production_common::account(
                7,
                "b",
                "operating_capital",
                1000,
            ));
            w.rebuild_account_index().unwrap();
            w.refresh_enterprise_views().unwrap();
        }
    });
    (w, basis)
}
fn cash(w: &WorldState, owner: &str, purpose: &str) -> i64 {
    w.accounts
        .iter()
        .filter(|a| a.owner_id == ExternalId::from(owner) && a.purpose == purpose)
        .map(|a| a.balance.0)
        .sum()
}

#[test]
fn actual_closing_pays_verified_dividend_and_tax_and_records_all_five_audits() {
    let (mut w, basis) = fixture(false);
    let r = common::request();
    let before_goods = w.stockpiles.clone();
    let receipt = execute_income_closing(&mut w, &r, &basis).unwrap();
    assert_eq!(cash(&w, "ar", "personal_savings"), 126);
    assert_eq!(cash(&w, "treasury:s", "operating_capital"), 15); // wage tax1 + dividend tax14.
    assert_eq!(
        w.income_data.producer_income[0]["net_income_minor"],
        json!("140")
    );
    assert!(receipt.invariants.all_required_passed());
    assert_eq!(w.stockpiles, before_goods);
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_income_closing(&mut w, &r, &basis).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn prepared_closing_rejects_changed_price_stock_policy_rights_order_or_transport() {
    for case in 0..6 {
        let (mut w, basis) = fixture(false);
        let r = common::request();
        let prepared = prepare_income_closing(&w, &r, &basis).unwrap();
        match case {
            0 => w.stockpiles[0].quantity += 1.,
            1 => {
                w.market_data
                    .price_calibration
                    .as_mut()
                    .unwrap()
                    .prices_by_currency
                    .get_mut("COIN")
                    .unwrap()
                    .insert("iron".into(), json!(2));
            }
            2 => {
                w.production_data.policies.taxes.insert("s".into(), 0.2);
            }
            3 => w.mobility_data.production_beneficiaries[0]["source"] = json!("changed"),
            4 => w.market_data.phase_receipts[0].orders[0].quantity += 1.,
            _ => w.transport_data.supply_routes.push(json!({"id":"changed"})),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(apply_income_closing(&mut w, prepared).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn late_artisan_recipient_failure_rolls_back_earlier_producer_dividend_and_metadata() {
    let (mut w, basis) = fixture(true);
    w.pops
        .iter_mut()
        .find(|p| p.external_id == ExternalId::from("q"))
        .unwrap()
        .currency_id = Some("FOREIGN".into());
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_income_closing(&mut w, &common::request(), &basis).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    assert_eq!(cash(&w, "ar", "personal_savings"), 0);
}
#[test]
fn deleted_tampered_duplicate_or_orphan_income_history_is_rejected() {
    for case in 0..4 {
        let (mut w, basis) = fixture(false);
        let r = common::request();
        let receipt = execute_income_closing(&mut w, &r, &basis).unwrap();
        match case {
            0 => w
                .production_data
                .committed_operations
                .retain(|op| op.value["type"] != json!("enterprise_dividend")),
            1 => {
                w.production_data
                    .committed_operations
                    .iter_mut()
                    .find(|op| op.value["type"] == json!("enterprise_dividend"))
                    .unwrap()
                    .value["credits"][0]["amount_minor"] = json!("999")
            }
            2 => w.income_data.phase_receipts.push(receipt),
            _ => w.income_data.phase_receipts.clear(),
        };
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_income_closing(&mut w, &r, &basis).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
#[test]
fn required_phase_loss_before_income_and_changed_request_never_pay() {
    let (mut w, basis) = fixture(false);
    w.budget_data.phase_receipts.clear();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_income_closing(&mut w, &common::request(), &basis).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let (mut w, basis) = fixture(false);
    let r = common::request();
    execute_income_closing(&mut w, &r, &basis).unwrap();
    let mut other = r.clone();
    other.policies.taxes.insert("s".into(), 0.2);
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_income_closing(&mut w, &other, &basis).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn native_roundtrip_retains_receipts_and_lawful_later_account_change_is_not_repaid() {
    let (mut w, basis) = fixture(false);
    let r = common::request();
    let receipt = execute_income_closing(&mut w, &r, &basis).unwrap();
    let mut w: WorldState = serde_json::from_value(serde_json::to_value(w).unwrap()).unwrap();
    w.rebuild_account_index().unwrap();
    w.accounts
        .iter_mut()
        .find(|a| a.owner_id == ExternalId::from("ar"))
        .unwrap()
        .balance
        .0 -= 1;
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_income_closing(&mut w, &r, &basis).unwrap(), receipt);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
fn opening_fixture() -> WorldState {
    let mut w = production_common::world();
    w.production_data.factories.clear();
    w.production_data.contracts.clear();
    let mut method = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    method.kind = "artisan".into();
    method.method_id = "artisan:steel".into();
    method.staffing = vec!["artisans".into()];
    w.production_data.catalog.as_mut().unwrap().methods = vec![method.clone()];
    method.method_id = "artisan:next".into();
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .push(method);
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"a","owner_id":"o","province_id":1,"facility_id":"wh","method_id":"artisan:steel","people_slots":0,"assignments":[],"pending_retooling":{"event_id":"previous-payment","method_id":"artisan:next","remaining_days":7,"ready_date":{"year":306,"month":1,"day":8}}})).unwrap());
    w.refresh_enterprise_views().unwrap();
    w
}
#[test]
fn opening_countdown_and_completion_are_atomic_replayable_and_no_second_payment() {
    let mut w = opening_fixture();
    let r = common::request();
    let first = execute_artisan_opening(&mut w, &r).unwrap();
    assert!(first.operations.is_empty());
    let before = serde_json::to_value(&w).unwrap();
    assert_eq!(execute_artisan_opening(&mut w, &r).unwrap(), first);
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut w: WorldState = serde_json::from_value(serde_json::to_value(w).unwrap()).unwrap();
    w.campaign.day = 8;
    let mut next = r.clone();
    next.period = "w2".into();
    let accounts = w.accounts.clone();
    let second = execute_artisan_opening(&mut w, &next).unwrap();
    assert_eq!(second.operations.len(), 1);
    assert_eq!(w.accounts, accounts);
    assert!(w.production_data.artisans[0].pending_retooling.is_none());
    assert_eq!(
        w.production_data.artisans[0].base.method_id,
        ExternalId::from("artisan:next")
    );
}

#[test]
fn duplicate_receipt_operations_are_rejected_even_with_duplicated_evidence() {
    let (mut w, basis) = fixture(false);
    let r = common::request();
    execute_income_closing(&mut w, &r, &basis).unwrap();
    let receipt = &mut w.income_data.phase_receipts[0];
    receipt.operations.push(receipt.operations[0].clone());
    receipt
        .operation_evidence
        .insert(1, receipt.operation_evidence[0].clone());
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_income_closing(&mut w, &r, &basis).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn opening_apply_cannot_publish_a_prepared_closing_phase() {
    let (mut w, basis) = fixture(false);
    let prepared = prepare_income_closing(&w, &common::request(), &basis).unwrap();
    let before = serde_json::to_value(&w).unwrap();
    assert!(kingmaker_sim::income_cycle::apply_artisan_opening(&mut w, prepared).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn empty_opening_receipt_loss_is_detected_before_countdown_is_decremented_again() {
    let mut w = opening_fixture();
    let r = common::request();
    execute_artisan_opening(&mut w, &r).unwrap();
    w.income_data.phase_receipts.clear();
    let before = serde_json::to_value(&w).unwrap();
    assert!(execute_artisan_opening(&mut w, &r).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}

#[test]
fn explicit_policy_override_preserves_inherited_terms_and_unknown_income_history() {
    let (mut w, basis) = fixture(false);
    w.production_data
        .policies
        .taxes
        .insert("other-state".into(), 0.3);
    w.production_data
        .policies
        .extra
        .insert("authored-term".into(), json!({"keep":7}));
    w.income_data.producer_income = vec![
        json!({"owner_id":"o","currency_id":"COIN","authored_note":{"keep":true}}),
        json!({"owner_id":"inactive","currency_id":"COIN","untouched":19}),
    ];
    let mut r = common::request();
    r.policies.taxes.insert("s".into(), 0.2);
    let receipt = execute_income_closing(&mut w, &r, &basis).unwrap();
    assert_eq!(receipt.effective_policies.taxes["s"], 0.2);
    assert_eq!(receipt.effective_policies.taxes["other-state"], 0.3);
    assert_eq!(
        receipt.effective_policies.extra["authored-term"],
        json!({"keep":7})
    );
    assert_eq!(cash(&w, "ar", "personal_savings"), 112);
    assert_eq!(
        w.income_data.producer_income[0]["authored_note"],
        json!({"keep":true})
    );
    assert_eq!(
        w.income_data.producer_income[1],
        json!({"owner_id":"inactive","currency_id":"COIN","untouched":19})
    );
}

#[test]
fn registry_or_evidence_tampering_is_rejected_without_repaying() {
    for case in 0..3 {
        let (mut w, basis) = fixture(false);
        let r = common::request();
        execute_income_closing(&mut w, &r, &basis).unwrap();
        let receipt = &mut w.income_data.phase_receipts[0];
        match case {
            0 => receipt.registry_changes.producer_income[0]["net_income_minor"] = json!("999"),
            1 => receipt.operation_evidence.clear(),
            _ => receipt.input_evidence.clear(),
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_income_closing(&mut w, &r, &basis).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}

#[test]
fn invalid_or_stale_provenance_cannot_be_published() {
    for case in 0..4 {
        let mut w = opening_fixture();
        let stock = w.stockpiles[0].external_id.clone();
        let quantity = w.stockpiles[0].quantity;
        w.income_data.stock_contributions = vec![StockContribution {
            stock_id: stock,
            producer_id: "a".into(),
            quantity,
        }];
        match case {
            0 => w.income_data.stock_contributions[0].producer_id = "missing".into(),
            1 => w.income_data.stock_contributions[0].quantity = quantity + 1.,
            2 => w.income_data.stock_contributions[0].stock_id = "missing".into(),
            _ => w
                .income_data
                .stock_contributions
                .push(w.income_data.stock_contributions[0].clone()),
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(execute_artisan_opening(&mut w, &common::request()).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
    let mut w = opening_fixture();
    let r = common::request();
    let prepared = kingmaker_sim::income_cycle::prepare_artisan_opening(&w, &r).unwrap();
    w.income_data.stock_contributions.push(StockContribution {
        stock_id: w.stockpiles[0].external_id.clone(),
        producer_id: "a".into(),
        quantity: 1.,
    });
    let before = serde_json::to_value(&w).unwrap();
    assert!(kingmaker_sim::income_cycle::apply_artisan_opening(&mut w, prepared).is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
