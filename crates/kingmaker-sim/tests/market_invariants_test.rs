mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::market::*;
use kingmaker_sim::{market_cycle::*, market_invariants::*};
#[test]
fn audit_rejects_free_goods_unexplained_money_and_changed_title() {
    let before = market_world();
    let mut after = before.clone();
    let req = MarketRequest {
        expected_revision: 20,
        period: "w".into(),
        stage: MarketStage::EarlyFood,
        profile: Default::default(),
        priorities: None,
    };
    let receipt = execute_market_phase(&mut after, &req).unwrap();
    let audit = check_market_operations(&before, &after, &receipt);
    assert!(audit.production_required_passed(), "{audit:?}");
    let mut wrong = after.clone();
    wrong.accounts[0].balance.0 += 1;
    assert!(!check_market_operations(&before, &wrong, &receipt)
        .money
        .is_passed());
    let mut wrong = after.clone();
    wrong.stockpiles[0].quantity += 1.0;
    assert!(!check_market_operations(&before, &wrong, &receipt)
        .goods
        .is_passed());
    let mut wrong = after.clone();
    wrong.stockpiles[0].owner_id = "thief".into();
    assert!(matches!(
        check_market_operations(&before, &wrong, &receipt).rights,
        kingmaker_core::audit::CheckStatus::Failed { .. }
    ));
}
