//! Regression tests for indexed application of prepared changes.
//! These exercise publication, not merely a stand-alone map implementation.
use super::*;

fn account(id: u32, owner: ExternalId, balance: i64) -> AccountBalance {
    AccountBalance {
        id: AccountId(id),
        external_id: format!("account-{id}").into(),
        owner_id: owner,
        currency_id: "coin".into(),
        purpose: "personal_savings".into(),
        balance: Money(balance),
    }
}

fn observed(world: &WorldState) -> Value {
    serde_json::to_value((
        &world.campaign,
        &world.accounts,
        &world.stockpiles,
        &world.production_data.storage_reservations,
        &world.production_data.obligations,
        &world.production_data.committed_operations,
    )).unwrap()
}

fn reject_without_publication(world: &mut WorldState, changes: PreparedChanges, reason: &str) {
    let before = observed(world);
    let err = apply_prepared(world, changes).unwrap_err();
    assert!(err.to_string().contains(reason), "unexpected error: {err}");
    assert_eq!(observed(world), before, "failed precondition published changes");
}

#[test]
fn perf_existing_account_precondition_accepts_exact_previous_value() {
    let mut world = WorldState::default();
    let old = account(1, "owner".into(), 100);
    world.accounts.push(old.clone());
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    let mut next = old.clone();
    next.balance = Money(90);
    changes.accounts.push((old.address(), Some(old), next.clone()));
    apply_prepared(&mut world, changes).unwrap();
    assert_eq!(world.accounts, vec![next]);
}

#[test]
fn perf_stale_account_without_revision_bump_rejects_all_changes() {
    let mut world = WorldState::default();
    let old = account(1, "owner".into(), 100);
    world.accounts.push(old.clone());
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    let mut next = old.clone();
    next.balance = Money(90);
    changes.accounts.push((old.address(), Some(old), next));
    world.accounts[0].balance = Money(99);
    reject_without_publication(&mut world, changes, "Prepared account precondition changed");
}

#[test]
fn perf_absent_account_precondition_rejects_intervening_insertion() {
    let mut world = WorldState::default();
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    let new = account(1, "owner".into(), 1);
    changes.accounts.push((new.address(), None, new.clone()));
    world.accounts.push(new);
    reject_without_publication(&mut world, changes, "Prepared account precondition changed");
}

#[test]
fn perf_account_precondition_rejects_intervening_deletion() {
    let mut world = WorldState::default();
    let old = account(1, "owner".into(), 100);
    world.accounts.push(old.clone());
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    changes.accounts.push((old.address(), Some(old.clone()), old));
    world.accounts.clear();
    reject_without_publication(&mut world, changes, "Prepared account precondition changed");
}

#[test]
fn perf_numeric_and_textual_owner_ids_remain_distinct() {
    let number = ExternalId::from_json(json!(7)).unwrap();
    let text = ExternalId::from_json(json!("7")).unwrap();
    let mut world = WorldState::default();
    let a = account(1, number, 100);
    let b = account(2, text, 200);
    world.accounts = vec![a.clone(), b.clone()];
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    let mut next = b.clone();
    next.balance = Money(180);
    changes.accounts.push((b.address(), Some(b), next.clone()));
    apply_prepared(&mut world, changes).unwrap();
    assert_eq!(world.accounts, vec![a, next]);
}

#[test]
fn perf_stock_precondition_rejects_changed_quantity() {
    let mut world = WorldState::default();
    let old = StockpileRecord { id: 1, external_id: "stock".into(), quantity: 3.0, ..Default::default() };
    world.stockpiles.push(old.clone());
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    let mut next = old.clone();
    next.quantity = 2.0;
    changes.stocks.push((old.external_id.clone(), Some(old), next));
    world.stockpiles[0].quantity = 4.0;
    reject_without_publication(&mut world, changes, "Prepared stock precondition changed");
}

#[test]
fn perf_stock_precondition_rejects_intervening_deletion() {
    let mut world = WorldState::default();
    let old = StockpileRecord { id: 1, external_id: "stock".into(), quantity: 3.0, ..Default::default() };
    world.stockpiles.push(old.clone());
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    changes.stocks.push((old.external_id.clone(), Some(old.clone()), old));
    world.stockpiles.clear();
    reject_without_publication(&mut world, changes, "Prepared stock precondition changed");
}

#[test]
fn perf_storage_precondition_still_checks_author_fields() {
    let mut world = WorldState::default();
    let old: StorageReservation = serde_json::from_value(json!({
        "id":"reservation", "owner_id":"owner", "facility_id":"warehouse",
        "goods_id":"iron", "unit":"ton", "status":"released", "quantity":0,
        "accepted_quantity":1.0, "released_quantity":1.0, "author_note":"original"
    })).unwrap();
    world.production_data.storage_reservations.push(old.clone());
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    changes.storage.push((old.id.clone(), Some(old.clone()), old.clone()));
    let mut json = serde_json::to_value(old).unwrap();
    json["author_note"] = json!("changed");
    world.production_data.storage_reservations[0] = serde_json::from_value(json).unwrap();
    reject_without_publication(&mut world, changes, "Prepared storage precondition changed");
}

#[test]
fn perf_obligation_precondition_still_checks_creditor_terms() {
    let mut world = WorldState::default();
    let old: Obligation = serde_json::from_value(json!({
        "obligation_id":"debt", "payer_id":"payer", "payee_id":"payee",
        "currency_id":"coin", "purpose":"production", "amount_minor":"123",
        "author_note":"original"
    })).unwrap();
    world.production_data.obligations.push(old.clone());
    let mut changes = OperationSession::new(&world).unwrap().finish().unwrap();
    changes.debts.push((old.obligation_id.clone(), Some(old.clone()), old));
    world.production_data.obligations[0].amount_minor = Money(124);
    reject_without_publication(&mut world, changes, "Prepared debt precondition changed");
}
