#[path = "../common/production.rs"]
mod common;
use crate::{construction_operations::stage_construction_operations, operation_session::*};
use kingmaker_core::{operations::*, *};
use serde_json::json;
fn contribution() -> EconomicOperation {
    serde_json::from_value(json!({"event_id":"investment:w:project:investor","type":"capitalist_investment","project_id":"project","investor_id":"investor","income_recognized":false,"debits":[{"owner_id":"o","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"10"}],"credits":[{"owner_id":"project:project","currency_id":"COIN","purpose":"construction_escrow","amount_minor":"8"},{"owner_id":"project:project","currency_id":"COIN","purpose":"factory_startup_reserve","amount_minor":"2"}]})).unwrap()
}
#[test]
fn private_construction_authority_conserves_money_and_only_creates_exact_credit_addresses() {
    let mut w = common::world();
    let before = w.accounts.iter().map(|a| a.balance.0).sum::<i64>();
    let mut s = OperationSession::new(&w).unwrap();
    stage_construction_operations(&mut s, &[contribution()]).unwrap();
    let changes = s.finish().unwrap();
    apply_prepared(&mut w, changes).unwrap();
    assert_eq!(w.accounts.iter().map(|a| a.balance.0).sum::<i64>(), before);
    for (purpose, value) in [("construction_escrow", 8), ("factory_startup_reserve", 2)] {
        assert_eq!(
            w.accounts
                .iter()
                .find(|a| a.owner_id == ExternalId::from("project:project") && a.purpose == purpose)
                .unwrap()
                .balance,
            Money(value)
        );
    }
}
#[test]
fn late_unbalanced_construction_payment_poisons_whole_private_plan() {
    let w = common::world();
    let op = contribution();
    let mut bad = op.clone();
    bad.event_id = "bad".into();
    bad.credits[0].amount_minor = Money(9);
    let mut s = OperationSession::new(&w).unwrap();
    assert!(stage_construction_operations(&mut s, &[op, bad]).is_err());
    assert!(s.finish().is_err());
}

#[test]
fn actual_private_material_consumption_preserves_money_and_cannot_create_goods() {
    let mut w=common::world();
    w.stockpiles[0].owner_id="project:p".into();
    let before=w.accounts.clone();
    let op:EconomicOperation=serde_json::from_value(json!({"event_id":"construction:materials","type":"construction_materials","project_id":"p","stock_changes":[{"stock_id":"iron","delta":-0.5}]})).unwrap();
    let mut s=OperationSession::new(&w).unwrap();stage_construction_operations(&mut s,&[op.clone()]).unwrap();let changes=s.finish().unwrap();apply_prepared(&mut w,changes).unwrap();
    assert!((w.stockpiles[0].quantity-0.6).abs()<1e-12);assert_eq!(w.accounts,before);
    for delta in [0.5,-9.] {
        let mut invalid=op.clone();invalid.event_id="invalid-material".into();invalid.stock_changes[0].delta=delta;
        let mut s=OperationSession::new(&w).unwrap();assert!(stage_construction_operations(&mut s,&[invalid]).is_err());assert!(s.finish().is_err());
    }
}
