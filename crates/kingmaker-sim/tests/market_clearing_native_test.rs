mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::*;
use kingmaker_sim::{market_clearing::*, operation_session::*};
use serde_json::json;
fn basic() -> Vec<String> {
    vec!["basic".into()]
}
#[test]
fn two_buyers_share_five_each_and_reversed_input_is_equal() {
    let mut w = market_world();
    w.stockpiles[0].quantity = 10.0;
    let s = OperationSession::new(&w).unwrap();
    let view = MarketView::from_session(&s).unwrap();
    let orders = vec![
        order("a", "a", "grain", 10.0, "basic", true),
        order("b", "b", "grain", 10.0, "basic", true),
    ];
    let r = plan_local_orders(&view, "w", &orders, &basic()).unwrap();
    let reverse = plan_local_orders(
        &view,
        "w",
        &orders.iter().rev().cloned().collect::<Vec<_>>(),
        &basic(),
    )
    .unwrap();
    assert_eq!(r.operations, reverse.operations);
    assert_eq!(r.fulfilled.get("a"), Some(&5.0));
    assert_eq!(r.fulfilled.get("b"), Some(&5.0));
    assert_eq!(w.stockpiles[0].quantity, 10.0);
}
#[test]
fn own_one_plus_paid_nine_preserves_two_pass_receipts() {
    let mut w = market_world();
    w.stockpiles[0].quantity = 9.0;
    let mut own = w.stockpiles[0].clone();
    own.id = 2;
    own.external_id = "own:a".into();
    own.owner_id = "a".into();
    own.quantity = 1.0;
    w.stockpiles.push(own);
    let s = OperationSession::new(&w).unwrap();
    let r = plan_local_orders(
        &MarketView::from_session(&s).unwrap(),
        "w",
        &[order("o", "a", "grain", 10.0, "basic", true)],
        &basic(),
    )
    .unwrap();
    assert_eq!(r.fulfilled.get("o"), Some(&10.0));
    let paid = r
        .operations
        .iter()
        .filter(|o| o.kind == kingmaker_core::operations::OperationKind::Purchase)
        .map(|o| {
            serde_json::from_value::<Money>(o.meta["paid_minor"].clone())
                .unwrap()
                .0
        })
        .sum::<i64>();
    assert_eq!(paid, 900);
}
#[test]
fn common_wallet_priority_and_no_imaginary_foreign_currency() {
    let mut w = market_world();
    w.accounts[0].balance = Money(500);
    let mut tools = w.stockpiles[0].clone();
    tools.id = 2;
    tools.external_id = "tools".into();
    tools.goods = Goods::from_external("tools".into());
    tools.quantity = 10.0;
    w.stockpiles.push(tools);
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("tools".into(), json!(1));
    let s = OperationSession::new(&w).unwrap();
    let view = MarketView::from_session(&s).unwrap();
    let r = plan_local_orders(
        &view,
        "w",
        &[
            order("food", "a", "grain", 10.0, "basic", true),
            order("tools", "a", "tools", 10.0, "basic", true),
        ],
        &basic(),
    )
    .unwrap();
    assert_eq!(r.fulfilled.get("food"), Some(&2.5));
    assert_eq!(r.fulfilled.get("tools"), Some(&2.5));
    w.stockpiles[0].currency_id = Some("FOREIGN".into());
    w.accounts.retain(|a| a.owner_id.as_str() != Some("seller"));
    let s = OperationSession::new(&w).unwrap();
    let r = plan_local_orders(
        &MarketView::from_session(&s).unwrap(),
        "w",
        &[order("food", "a", "grain", 1.0, "basic", true)],
        &basic(),
    )
    .unwrap();
    assert!(r
        .operations
        .iter()
        .any(|o| o.meta.get("reason") == Some(&json!("unknown_settlement_or_fx"))));
}

#[test]
fn priorities_rounding_and_fallback_match_actual_js() {
    for variant in 0..5 {
        let mut w = market_world();
        w.accounts[0].balance = Money(variant * 101);
        w.stockpiles[0].quantity = 3.03;
        let mut fish = w.stockpiles[0].clone();
        fish.id = 2;
        fish.external_id = "fish:2".into();
        fish.goods = Goods::from_external("fish".into());
        fish.quantity = 4.0;
        w.stockpiles.push(fish);
        let mut food = order("food", "a", "grain", 10.0, "basic", true);
        food.food_alternatives = vec![
            json!({"goods_id":"grain","share":0.5}),
            json!({"goods_id":"fish","share":0.5}),
        ];
        food.food_gross_quantity = Some(10.0);
        let mut other = order("other", "a", "fish", 2.0, "everyday", true);
        other.need_id = Some("fish".into());
        let priorities = if variant % 2 == 0 {
            vec!["basic".into(), "everyday".into()]
        } else {
            vec!["everyday".into(), "basic".into()]
        };
        let orders = vec![food, other];
        let session = OperationSession::new(&w).unwrap();
        let actual = plan_local_orders(
            &MarketView::from_session(&session).unwrap(),
            "w",
            &orders,
            &priorities,
        )
        .unwrap();
        let expected:Vec<kingmaker_core::operations::EconomicOperation>=serde_json::from_value(oracle(json!({"action":"clear","snapshot":snapshot(&w),"period":"w","orders":orders,"priorities":priorities}))).unwrap();
        assert_source_values(
            &serde_json::to_value(actual.operations).unwrap(),
            &serde_json::to_value(expected).unwrap(),
        );
    }
}

#[test]
fn military_food_is_not_eaten_twice_and_sale_enables_currency() {
    let mut w = market_world();
    let mut military = w.stockpiles[0].clone();
    military.id = 2;
    military.external_id = "military".into();
    military.goods = Goods::from_external("military_supply".into());
    military.quantity = 5.0;
    w.stockpiles.push(military);
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("military_supply".into(), json!(1));
    let mut food = order("food", "a", "grain", 5.0, "basic", true);
    food.food_gross_quantity = Some(10.0);
    food.military_food_substitution = Some(5.0);
    food.food_alternatives = vec![json!({"goods_id":"grain","share":1})];
    let mut military = order("supply", "a", "military_supply", 5.0, "basic", true);
    military.need_id = Some("military_supply".into());
    let session = OperationSession::new(&w).unwrap();
    let orders = vec![food, military];
    let actual = plan_local_orders(
        &MarketView::from_session(&session).unwrap(),
        "w",
        &orders,
        &basic(),
    )
    .unwrap();
    assert_eq!(actual.fulfilled.get("food"), Some(&5.0));
    assert!(!actual.fulfilled.contains_key("fallback:a:food:grain"));
    let expected:Vec<kingmaker_core::operations::EconomicOperation>=serde_json::from_value(oracle(json!({"action":"clear","snapshot":snapshot(&w),"period":"w","orders":orders,"priorities":basic()}))).unwrap();
    assert_source_values(
        &serde_json::to_value(actual.operations).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
    // A sale in the first priority establishes a new settlement currency even with a zero credit.
    w.accounts.retain(|a| a.owner_id.as_str() != Some("seller"));
    w.stockpiles[1].currency_id = Some("FOREIGN".into());
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("grain".into(), json!(0.001));
    let mut small = order("small", "a", "grain", 1.0, "basic", true);
    small.need_id = None;
    let mut next = order("next", "a", "military_supply", 1.0, "everyday", true);
    next.need_id = None;
    let orders = vec![small, next];
    let priorities = vec!["basic".into(), "everyday".into()];
    let session = OperationSession::new(&w).unwrap();
    let r = plan_local_orders(
        &MarketView::from_session(&session).unwrap(),
        "w",
        &orders,
        &priorities,
    )
    .unwrap();
    assert_eq!(r.fulfilled.get("next"), None); // source zero-price-rounded sale emits no credits
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("grain".into(), json!(1));
    let session = OperationSession::new(&w).unwrap();
    let r = plan_local_orders(
        &MarketView::from_session(&session).unwrap(),
        "w",
        &orders,
        &priorities,
    )
    .unwrap();
    assert_eq!(r.fulfilled.get("next"), Some(&1.0));
    let expected:Vec<kingmaker_core::operations::EconomicOperation>=serde_json::from_value(oracle(json!({"action":"clear","snapshot":snapshot(&w),"period":"w","orders":orders,"priorities":priorities}))).unwrap();
    assert_source_values(
        &serde_json::to_value(r.operations).unwrap(),
        &serde_json::to_value(expected).unwrap(),
    );
}
