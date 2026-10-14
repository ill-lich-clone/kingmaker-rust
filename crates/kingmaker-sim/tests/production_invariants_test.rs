#[path = "common/production.rs"]
mod common;
use kingmaker_core::*;
use kingmaker_sim::{
    invariants::{check_production_operations, CheckStatus},
    production::*,
};
#[test]
fn production_audit_explains_real_changes_and_detects_corruption() {
    let before = common::world();
    let mut after = before.clone();
    let r = execute_production_cycle(&mut after, &common::request()).unwrap();
    let audit = check_production_operations(&before, &after, &r);
    assert!(audit.money.is_passed());
    assert!(audit.goods.is_passed());
    assert!(audit.people.is_passed());
    assert!(audit.labor.is_passed());
    assert!(matches!(audit.rights, CheckStatus::NotMeasured { .. }));
    assert!(!audit.all_required_passed());
    assert_eq!(r.planned_outputs[0].quantity, 1.0);
    assert_eq!(r.registry_changes.storage.len(), 1);
    let mut damaged = after.clone();
    damaged.stockpiles[2].quantity += 1.0;
    assert!(matches!(
        check_production_operations(&before, &damaged, &r).goods,
        CheckStatus::Failed { .. }
    ));
    let mut damaged = after.clone();
    damaged.accounts[0].balance += Money(1);
    assert!(matches!(
        check_production_operations(&before, &damaged, &r).money,
        CheckStatus::Failed { .. }
    ));
    let mut damaged = after.clone();
    damaged.pops[0].size += 1;
    assert!(matches!(
        check_production_operations(&before, &damaged, &r).people,
        CheckStatus::Failed { .. }
    ));
    let mut damaged = after.clone();
    damaged.production_data.factories[0].tenure_days = Some(99.0);
    assert!(matches!(
        check_production_operations(&before, &damaged, &r).labor,
        CheckStatus::Failed { .. }
    ));
}
