use kingmaker_core::{AccountAddress, AccountBalance, AccountId, ExternalId, Money};
use kingmaker_sim::ledger::{LedgerSession, TransferRequest};
fn account(id: u32, balance: i64) -> AccountBalance {
    AccountBalance {
        id: AccountId(id),
        external_id: format!("account:{id}").into(),
        owner_id: format!("owner:{id}").into(),
        currency_id: "COIN".into(),
        purpose: "personal_savings".into(),
        balance: Money(balance),
    }
}
fn request(amount: i64) -> TransferRequest {
    let address = |owner: &str| AccountAddress {
        owner_id: owner.into(),
        currency_id: "COIN".into(),
        purpose: "personal_savings".into(),
    };
    TransferRequest {
        event_id: "payroll-fixture".into(),
        from: address("owner:1"),
        to: address("owner:2"),
        amount: Money(amount),
        reason: "payroll".into(),
    }
}
#[test]
fn test_ledger_transaction_conserves_total_cash() {
    let mut ledger = LedgerSession::from_accounts(&[account(1, 10000), account(2, 5000)]).unwrap();
    let initial = ledger.totals_by_currency()[&ExternalId::from("COIN")];
    assert_eq!(initial, 15000);
    ledger
        .transfer_once(request(2500))
        .expect("Transfer should succeed");
    assert_eq!(ledger.get_balance(AccountId(1)), Some(Money(7500)));
    assert_eq!(ledger.get_balance(AccountId(2)), Some(Money(7500)));
    assert_eq!(
        ledger.totals_by_currency()[&ExternalId::from("COIN")],
        initial
    );
}
#[test]
fn test_ledger_rejects_insufficient_funds() {
    let mut ledger = LedgerSession::from_accounts(&[account(1, 100), account(2, 0)]).unwrap();
    assert!(ledger.transfer_once(request(500)).is_err());
    assert_eq!(ledger.get_balance(AccountId(1)), Some(Money(100)));
    assert_eq!(ledger.get_balance(AccountId(2)), Some(Money(0)));
}
