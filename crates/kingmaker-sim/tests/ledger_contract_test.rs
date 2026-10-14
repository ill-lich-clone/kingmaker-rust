use kingmaker_core::{AccountAddress, AccountBalance, AccountId, ExternalId, Money};
use kingmaker_sim::ledger::{LedgerSession, TransferOutcome, TransferRequest};
fn account(id: u32, owner: &str, currency: &str, balance: i64) -> AccountBalance {
    AccountBalance {
        id: AccountId(id),
        external_id: ExternalId::from(format!("account:{id}")),
        owner_id: owner.into(),
        currency_id: currency.into(),
        purpose: "personal_savings".into(),
        balance: Money(balance),
    }
}
fn address(owner: &str, currency: &str) -> AccountAddress {
    AccountAddress {
        owner_id: owner.into(),
        currency_id: currency.into(),
        purpose: "personal_savings".into(),
    }
}
fn session() -> LedgerSession {
    LedgerSession::from_accounts(&[
        account(1, "one", "COIN", 100),
        account(2, "two", "COIN", 100),
    ])
    .unwrap()
}
fn request(id: &str, amount: i64) -> TransferRequest {
    TransferRequest {
        event_id: id.into(),
        from: address("one", "COIN"),
        to: address("two", "COIN"),
        amount: Money(amount),
        reason: "fixture".into(),
    }
}
#[test]
fn real_transfer_and_retry_preserve_balances_and_history() {
    let mut s = session();
    assert_eq!(
        s.transfer_once(request("op", 50)).unwrap(),
        TransferOutcome::Applied
    );
    assert_eq!(s.get_balance(AccountId(1)), Some(Money(50)));
    assert_eq!(s.get_balance(AccountId(2)), Some(Money(150)));
    assert_eq!(
        s.transfer_once(request("op", 50)).unwrap(),
        TransferOutcome::AlreadyApplied
    );
    assert_eq!(s.get_balance(AccountId(1)), Some(Money(50)));
    assert_eq!(s.transactions().len(), 1);
    assert!(s.transfer_once(request("op", 51)).is_err());
    assert_eq!(s.transactions().len(), 1);
}
#[test]
fn self_transfer_creates_no_money_and_is_recorded_as_noop() {
    let mut s = session();
    let mut r = request("self", 50);
    r.to = r.from.clone();
    s.transfer_once(r.clone()).unwrap();
    assert_eq!(s.get_balance(AccountId(1)), Some(Money(100)));
    assert!(s.transactions()[0].noop);
    assert_eq!(s.transfer_once(r).unwrap(), TransferOutcome::AlreadyApplied);
    assert_eq!(s.transactions().len(), 1);
}
#[test]
fn failed_transfers_are_atomic_and_never_create_accounts() {
    for (name, mut r) in [
        ("negative", request("negative", -50)),
        ("missing", request("missing", 50)),
        ("purpose", request("purpose", 50)),
        ("insufficient", request("insufficient", 200)),
    ] {
        let mut s = session();
        if name == "missing" {
            r.to.owner_id = "unknown".into();
        }
        if name == "purpose" {
            r.from.purpose = "operating_capital".into();
        }
        assert!(s.transfer_once(r).is_err(), "{name}");
        assert_eq!(s.get_balance(AccountId(1)), Some(Money(100)));
        assert_eq!(s.get_balance(AccountId(2)), Some(Money(100)));
        assert_eq!(s.get_balance(AccountId(999)), None);
        assert!(s.transactions().is_empty());
    }
}
#[test]
fn zero_transfer_still_requires_real_accounts() {
    let mut s = session();
    let mut r = request("zero", 0);
    r.to.owner_id = "unknown".into();
    assert!(s.transfer_once(r).is_err());
    assert!(s.transactions().is_empty());
    assert_eq!(s.get_balance(AccountId(1)), Some(Money(100)));
}
#[test]
fn different_currencies_cannot_be_transferred_or_net_compensated() {
    let mut s = LedgerSession::from_accounts(&[
        account(1, "one", "COIN", 100),
        account(2, "two", "OTHER", 100),
    ])
    .unwrap();
    let mut r = request("fx", 50);
    r.to.currency_id = "OTHER".into();
    assert!(s.transfer_once(r).is_err());
    assert_eq!(s.get_balance(AccountId(1)), Some(Money(100)));
    assert_eq!(s.get_balance(AccountId(2)), Some(Money(100)));
}
#[test]
fn recipient_overflow_does_not_debit_sender_or_panic() {
    let mut s = LedgerSession::from_accounts(&[
        account(1, "one", "COIN", 100),
        account(2, "two", "COIN", i64::MAX),
    ])
    .unwrap();
    assert!(s.transfer_once(request("overflow", 1)).is_err());
    assert_eq!(s.get_balance(AccountId(1)), Some(Money(100)));
    assert_eq!(s.get_balance(AccountId(2)), Some(Money(i64::MAX)));
    assert!(s.transactions().is_empty());
}
#[test]
fn replay_is_checked_before_current_funds() {
    let mut s = session();
    s.transfer_once(request("all", 100)).unwrap();
    assert_eq!(s.get_balance(AccountId(1)), Some(Money(0)));
    assert_eq!(
        s.transfer_once(request("all", 100)).unwrap(),
        TransferOutcome::AlreadyApplied
    );
    assert_eq!(s.transactions().len(), 1);
}
