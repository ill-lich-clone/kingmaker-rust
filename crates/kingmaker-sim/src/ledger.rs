use kingmaker_core::{AccountAddress, AccountBalance, AccountId, ExternalId, Money};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum LedgerError {
    #[error("Account not found: {0:?}")]
    AccountNotFound(AccountAddress),
    #[error("Duplicate account ID/address")]
    DuplicateAccount,
    #[error("Invalid empty financial address")]
    InvalidAddress,
    #[error("Negative amount")]
    NegativeAmount,
    #[error("Unverified negative initial balance: {0:?}")]
    NegativeBalance(AccountId),
    #[error("Currencies differ")]
    CurrencyMismatch,
    #[error("Insufficient authorized purpose cash")]
    InsufficientFunds,
    #[error("Amount overflow")]
    Overflow,
    #[error("Empty event ID")]
    EmptyEvent,
    #[error("Event ID already used for a different operation: {0}")]
    EventConflict(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    pub event_id: String,
    pub from: AccountAddress,
    pub to: AccountAddress,
    pub amount: Money,
    pub reason: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferOutcome {
    Applied,
    AlreadyApplied,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub request: TransferRequest,
    pub from: AccountId,
    pub to: AccountId,
    pub noop: bool,
}
#[derive(Debug, Clone, Default)]
pub struct LedgerSession {
    accounts: HashMap<AccountId, AccountBalance>,
    addresses: HashMap<AccountAddress, AccountId>,
    events: HashMap<String, usize>,
    history: Vec<Transaction>,
}
impl LedgerSession {
    pub fn from_accounts(accounts: &(impl kingmaker_core::row_store::ReadSequence<AccountBalance> + ?Sized)) -> Result<Self, LedgerError> {
        let mut session = Self::default();
        for account in accounts.iter() {
            let address = account.address();
            if address.purpose.is_empty()
                || [&address.owner_id, &address.currency_id]
                    .iter()
                    .any(|id| id.as_str() == Some(""))
            {
                return Err(LedgerError::InvalidAddress);
            }
            if account.balance.units() < 0 {
                return Err(LedgerError::NegativeBalance(account.id));
            }
            if session
                .accounts
                .insert(account.id, account.clone())
                .is_some()
                || session.addresses.insert(address, account.id).is_some()
            {
                return Err(LedgerError::DuplicateAccount);
            }
        }
        Ok(session)
    }
    pub fn get_balance(&self, id: AccountId) -> Option<Money> {
        self.accounts.get(&id).map(|a| a.balance)
    }
    pub fn totals_by_currency(&self) -> HashMap<ExternalId, i128> {
        let mut totals = HashMap::new();
        for a in self.accounts.values() {
            *totals.entry(a.currency_id.clone()).or_insert(0) += i128::from(a.balance.units());
        }
        totals
    }
    pub fn transfer_once(
        &mut self,
        request: TransferRequest,
    ) -> Result<TransferOutcome, LedgerError> {
        if request.event_id.is_empty() {
            return Err(LedgerError::EmptyEvent);
        }
        if let Some(&i) = self.events.get(&request.event_id) {
            return if self.history[i].request == request {
                Ok(TransferOutcome::AlreadyApplied)
            } else {
                Err(LedgerError::EventConflict(request.event_id))
            };
        }
        if request.amount.units() < 0 {
            return Err(LedgerError::NegativeAmount);
        }
        let from = *self
            .addresses
            .get(&request.from)
            .ok_or_else(|| LedgerError::AccountNotFound(request.from.clone()))?;
        let to = *self
            .addresses
            .get(&request.to)
            .ok_or_else(|| LedgerError::AccountNotFound(request.to.clone()))?;
        if request.from.currency_id != request.to.currency_id {
            return Err(LedgerError::CurrencyMismatch);
        }
        let old_from = self.accounts[&from].balance;
        let old_to = self.accounts[&to].balance;
        if old_from < request.amount {
            return Err(LedgerError::InsufficientFunds);
        }
        let noop = from == to || request.amount == Money::ZERO;
        // Complete every check before the first write; no rollback/panic path required.
        let new_from = old_from
            .checked_sub(request.amount)
            .ok_or(LedgerError::Overflow)?;
        let new_to = if noop {
            old_to
        } else {
            old_to
                .checked_add(request.amount)
                .ok_or(LedgerError::Overflow)?
        };
        if !noop {
            self.accounts
                .get_mut(&from)
                .expect("resolved account")
                .balance = new_from;
            self.accounts
                .get_mut(&to)
                .expect("resolved account")
                .balance = new_to;
        }
        self.events
            .insert(request.event_id.clone(), self.history.len());
        self.history.push(Transaction {
            request,
            from,
            to,
            noop,
        });
        Ok(TransferOutcome::Applied)
    }
    pub fn transactions(&self) -> &[Transaction] {
        &self.history
    }
}
