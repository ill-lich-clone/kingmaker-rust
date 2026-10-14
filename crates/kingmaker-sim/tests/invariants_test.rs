use kingmaker_core::*;
use kingmaker_sim::invariants::{
    check_conservation, check_with_operations, CheckStatus, OperationEvidence,
};
use kingmaker_sim::ledger::{LedgerSession, TransferRequest};
fn world() -> WorldState {
    let mut w = WorldState::default();
    w.provinces.push(ProvinceRecord {
        id: ProvinceId(1),
        pop_range: (0, 1),
        ..Default::default()
    });
    w.pops.push(PopRecord {
        id: PopId(1),
        external_id: "pop:one".into(),
        province_id: ProvinceId(1),
        size: 100,
        workforce: 60.0,
        employed: 30.0,
        unemployed: 30.0,
        ..Default::default()
    });
    w.stockpiles.push(StockpileRecord {
        id: 1,
        external_id: "stock:one".into(),
        owner_id: "one".into(),
        province_id: ProvinceId(1),
        goods: Goods::Grain,
        quantity: 100.0,
        ..Default::default()
    });
    for (id, owner) in [(1, "one"), (2, "two")] {
        w.accounts.push(AccountBalance {
            id: AccountId(id),
            external_id: format!("account:{id}").into(),
            owner_id: owner.into(),
            currency_id: "COIN".into(),
            purpose: "personal_savings".into(),
            balance: Money(100),
        });
    }
    w.rebuild_account_index().unwrap();
    w
}
#[test]
fn unexplained_people_and_goods_changes_are_failures() {
    let before = world();
    let mut after = before.clone();
    after.pops[0].size = 0;
    assert!(matches!(
        check_conservation(&before, &after).people,
        CheckStatus::Failed { .. }
    ));
    let mut after = before.clone();
    after.stockpiles[0].quantity += 1000.0;
    assert!(matches!(
        check_conservation(&before, &after).goods,
        CheckStatus::Failed { .. }
    ));
}
#[test]
fn currency_losses_are_not_cancelled_by_other_currency_credits() {
    let mut before = world();
    before.accounts[1].currency_id = "OTHER".into();
    before.rebuild_account_index().unwrap();
    let mut after = before.clone();
    after.accounts[0].balance = Money(50);
    after.accounts[1].balance = Money(150);
    assert!(matches!(
        check_conservation(&before, &after).money,
        CheckStatus::Failed { .. }
    ));
}
#[test]
fn unexplained_title_and_invalid_labor_are_failures() {
    let before = world();
    let mut after = before.clone();
    after.stockpiles[0].owner_id = "other_owner".into();
    assert!(matches!(
        check_conservation(&before, &after).rights,
        CheckStatus::Failed { .. }
    ));
    let mut after = before.clone();
    after.pops[0].employed = 80.0;
    assert!(matches!(
        check_conservation(&before, &after).labor,
        CheckStatus::Failed { .. }
    ));
}
#[test]
fn unsupported_sources_are_not_marked_ok_or_forbidden_demography() {
    let before = world();
    let mut after = before.clone();
    after.pops[0].size = 110;
    let r = check_with_operations(
        &before,
        &after,
        &[OperationEvidence::Unsupported {
            kind: "demographic_source".into(),
        }],
    );
    assert!(matches!(r.people, CheckStatus::NotMeasured { .. }));
    assert!(!r.all_required_passed());
}
#[test]
fn implemented_ledger_receipts_explain_account_deltas() {
    let before = world();
    let mut ledger = LedgerSession::from_accounts(&before.accounts).unwrap();
    ledger
        .transfer_once(TransferRequest {
            event_id: "op".into(),
            from: before.accounts[0].address(),
            to: before.accounts[1].address(),
            amount: Money(50),
            reason: "fixture".into(),
        })
        .unwrap();
    let mut after = before.clone();
    after.accounts[0].balance = Money(50);
    after.accounts[1].balance = Money(150);
    let tx = ledger.transactions()[0].clone();
    let r = check_with_operations(
        &before,
        &after,
        &[
            OperationEvidence::Transfer(tx.clone()),
            OperationEvidence::Transfer(tx),
        ],
    );
    assert!(matches!(r.money, CheckStatus::Passed));
    assert!(matches!(
        check_conservation(&before, &after).money,
        CheckStatus::Failed { .. }
    ));
}
#[test]
fn complete_unimplemented_rights_and_labor_checks_are_explicit() {
    let w = world();
    let r = check_conservation(&w, &w);
    assert!(matches!(r.money, CheckStatus::Passed));
    assert!(matches!(r.goods, CheckStatus::Passed));
    assert!(matches!(r.labor, CheckStatus::NotMeasured { .. }));
    assert!(matches!(r.rights, CheckStatus::NotMeasured { .. }));
    assert!(!r.all_required_passed());
}
