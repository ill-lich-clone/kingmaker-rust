mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::ProvinceId;
use kingmaker_sim::supply::{find_supply_route, SupplyNetwork};
use kingmaker_sim::{market_clearing::*, operation_session::*};

#[test]
fn test_market_clearing_strictly_respects_priorities() {
    let mut w = market_world();
    w.stockpiles[0].quantity = 100.0;
    let s = OperationSession::new(&w).unwrap();
    let r = plan_local_orders(
        &MarketView::from_session(&s).unwrap(),
        "w",
        &[
            order("everyday", "a", "grain", 50.0, "everyday", true),
            order("basic", "b", "grain", 80.0, "basic", true),
        ],
        &["basic".into(), "everyday".into()],
    )
    .unwrap();
    assert_eq!(r.fulfilled.get("basic"), Some(&80.0));
    assert_eq!(r.fulfilled.get("everyday"), Some(&20.0));
    assert_eq!(
        r.operations
            .iter()
            .map(|o| o.debits.iter().map(|l| l.amount_minor.0).sum::<i64>())
            .sum::<i64>(),
        10000
    );
}

#[test]
fn test_supply_network_finds_path() {
    let mut network = SupplyNetwork::default();
    network.add_route(ProvinceId(1), ProvinceId(2), 10.0); // 1 -> 2 distance 10
    network.add_route(ProvinceId(2), ProvinceId(3), 15.0); // 2 -> 3 distance 15

    let route = find_supply_route(&network, ProvinceId(1), ProvinceId(3));
    assert!(route.is_some());
    let path = route.unwrap();
    assert_eq!(
        path.provinces,
        vec![ProvinceId(1), ProvinceId(2), ProvinceId(3)]
    );
    assert_eq!(path.total_cost, 25.0);
}
