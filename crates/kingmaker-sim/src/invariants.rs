use kingmaker_core::row_store::ReadSequence;
use kingmaker_core::world_read::WorldRead;
use crate::ledger::{LedgerSession, Transaction};
use std::collections::HashMap;

pub use crate::production_invariants::check_production_operations;
pub use kingmaker_core::audit::{CheckStatus, InvariantReport};
#[derive(Debug, Clone)]
pub enum OperationEvidence {
    Transfer(Transaction),
    Unsupported { kind: String },
}
pub fn check_conservation(before: &dyn WorldRead, after: &dyn WorldRead) -> InvariantReport {
    check_with_operations(before, after, &[])
}
pub fn check_with_operations(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    operations: &(impl ReadSequence<OperationEvidence> + ?Sized),
) -> InvariantReport {
    let unsupported: Vec<_> = operations
        .iter()
        .filter_map(|o| match o {
            OperationEvidence::Unsupported { kind } => Some(kind.as_str()),
            _ => None,
        })
        .collect();
    if !unsupported.is_empty() {
        let status = unknown(&format!(
            "Operation validators not implemented: {}",
            unsupported.join(", ")
        ));
        return InvariantReport {
            money: status.clone(),
            goods: status.clone(),
            people: status.clone(),
            labor: status.clone(),
            rights: status,
        };
    }
    let money = check_money(before, after, operations);
    let before_people: HashMap<_, _> = before.pops().values()
        .map(|p| (&p.external_id, (p.size, p.workforce)))
        .collect();
    let after_people: HashMap<_, _> = after.pops().values()
        .map(|p| (&p.external_id, (p.size, p.workforce)))
        .collect();
    let people = if before_people == after_people
        && before_people.len() == before.pops().len()
        && after_people.len() == after.pops().len()
    {
        CheckStatus::Passed
    } else {
        failure("Unexplained people/workforce change")
    };
    let before_goods: HashMap<_, _> = before.stockpiles().values()
        .map(|s| (s.id, (&s.external_id, &s.goods, s.quantity, &s.unit)))
        .collect();
    let after_goods: HashMap<_, _> = after.stockpiles().values()
        .map(|s| (s.id, (&s.external_id, &s.goods, s.quantity, &s.unit)))
        .collect();
    let goods = if before_goods == after_goods
        && before_goods.len() == before.stockpiles().len()
        && after_goods.len() == after.stockpiles().len()
        && after.stockpiles().values()
            .all(|s| s.quantity.is_finite() && s.quantity >= 0.0)
    {
        CheckStatus::Passed
    } else {
        failure("Unexplained goods/units/stock change")
    };
    let validation = after.validate_links();
    let labor = if validation
        .errors
        .iter()
        .any(|e| e.contains("labor quantities"))
    {
        failure("Invalid employment/workforce quantities")
    } else if before.pops().values()
        .map(|p| (&p.external_id, p.employed, p.unemployed))
        .collect::<Vec<_>>()
        != after.pops().values()
            .map(|p| (&p.external_id, p.employed, p.unemployed))
            .collect::<Vec<_>>()
    {
        failure("Unexplained labor change")
    } else {
        unknown("Full employment assignment and double-use validator not implemented")
    };
    let before_titles: HashMap<_, _> = before.stockpiles().values()
        .map(|s| {
            (
                s.id,
                (
                    &s.owner_id,
                    s.province_id,
                    &s.facility_id,
                    &s.beneficiary_pop_id,
                ),
            )
        })
        .collect();
    let after_titles: HashMap<_, _> = after.stockpiles().values()
        .map(|s| {
            (
                s.id,
                (
                    &s.owner_id,
                    s.province_id,
                    &s.facility_id,
                    &s.beneficiary_pop_id,
                ),
            )
        })
        .collect();
    let before_account_titles: HashMap<_, _> = before.accounts().values()
        .map(|a| (a.id, a.address()))
        .collect();
    let after_account_titles: HashMap<_, _> =
        after.accounts().values().map(|a| (a.id, a.address())).collect();
    let rights = if before_titles != after_titles
        || before_account_titles != after_account_titles
        || !validation.errors.is_empty()
    {
        failure("Unexplained title/location/beneficiary change or broken reference")
    } else {
        unknown("Complete legal title, lease and reservation validators not implemented")
    };
    InvariantReport {
        money,
        goods,
        people,
        labor,
        rights,
    }
}
fn check_money(
    before: &dyn WorldRead,
    after: &dyn WorldRead,
    operations: &(impl ReadSequence<OperationEvidence> + ?Sized),
) -> CheckStatus {
    let mut ledger = match LedgerSession::from_accounts(&before.accounts()) {
        Ok(s) => s,
        Err(e) => return failure(&e.to_string()),
    };
    for evidence in operations.iter() {
        if let OperationEvidence::Transfer(transaction) = evidence {
            if let Err(e) = ledger.transfer_once(transaction.request.clone()) {
                return failure(&e.to_string());
            }
            let verified = ledger
                .transactions()
                .iter()
                .find(|t| t.request.event_id == transaction.request.event_id);
            if verified != Some(transaction) {
                return failure("Invalid transfer receipt account/amount/no-op provenance");
            }
        }
    }
    if before.accounts().len() != after.accounts().len() {
        return failure("Unexplained account creation/deletion");
    }
    let originals: HashMap<_, _> = before.accounts().values().map(|a| (a.id, a)).collect();
    let mut seen = std::collections::HashSet::new();
    for a in after.accounts().values() {
        if !seen.insert(a.id) {
            return failure("Duplicate account ID");
        }
        let Some(original) = originals.get(&a.id) else {
            return failure("Unknown account");
        };
        if original.address() != a.address() || original.external_id != a.external_id {
            return failure("Account identity/currency/purpose changed");
        }
        if ledger.get_balance(a.id) != Some(a.balance) {
            return failure(
                "Unexplained account balance change; currencies cannot offset each other",
            );
        }
    }
    CheckStatus::Passed
}
fn failure(reason: &str) -> CheckStatus {
    CheckStatus::Failed {
        reason: reason.into(),
    }
}
fn unknown(reason: &str) -> CheckStatus {
    CheckStatus::NotMeasured {
        reason: reason.into(),
    }
}
