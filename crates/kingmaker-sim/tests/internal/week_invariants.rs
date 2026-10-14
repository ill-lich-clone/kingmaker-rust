use crate::week_invariants::verify_week;
use kingmaker_core::*;
#[test]
fn equal_quantity_does_not_hide_a_changed_goods_identity_or_unit() {
    let mut before = WorldState::default();
    before.stockpiles.push(StockpileRecord {
        id: 1,
        external_id: "stock".into(),
        owner_id: "owner".into(),
        goods: Goods::Iron,
        quantity: 1.,
        unit: Some(serde_json::json!("t")),
        ..Default::default()
    });
    for unit in [false, true] {
        let mut after = before.clone();
        if unit {
            after.stockpiles[0].unit = Some(serde_json::json!("kg"));
        } else {
            after.stockpiles[0].goods = Goods::Coal;
        }
        assert!(verify_week(&before, &after, &[]).is_err());
    }
}
#[test]
fn extra_money_and_unrecorded_stock_are_not_accepted_as_a_healthy_week() {
    let before = WorldState::default();
    let mut after = before.clone();
    after.accounts.push(AccountBalance {
        id: AccountId(1),
        owner_id: "owner".into(),
        currency_id: "X".into(),
        purpose: "personal_savings".into(),
        balance: Money(1),
        ..Default::default()
    });
    assert!(verify_week(&before, &after, &[]).is_err());
    let mut after = before.clone();
    after.stockpiles.push(StockpileRecord {
        id: 1,
        external_id: "created".into(),
        owner_id: "owner".into(),
        quantity: 1.,
        ..Default::default()
    });
    assert!(verify_week(&before, &after, &[]).is_err());
}
#[test]
fn public_operation_payload_cannot_authorize_a_week_boundary() {
    let w = WorldState::default();
    let mut session = crate::operation_session::OperationSession::new(&w).unwrap();
    let op=serde_json::from_value(serde_json::json!({"event_id":"fake","type":"native_week_boundary","receipt_sha256":"fake"})).unwrap();
    assert!(session.stage(&op).is_err());
}
