#[path = "common/market.rs"]
mod market;
#[path = "common/allocation_meter.rs"]
mod meter;
#[global_allocator]
static ALLOC: meter::Allocator = meter::Allocator;
use kingmaker_core::{market::MarketData, ProvinceId};
use kingmaker_sim::{land_paths::LandPathIndex, market_index::OrderIndex, market_policy::MarketScope};
use serde_json::json;

#[test]
fn directed_paths_and_order_indexes_preserve_authored_choices_with_bounded_reuse() {
    let _lock = meter::MEASURE.lock().unwrap();
    let mut w = market::market_world();
    let mut orders = vec![market::order("a", "a", "flour", 2.0, "basic", true); 6];
    orders[1].quantity = 0.0;
    orders[2].province_id = ProvinceId(2);
    orders[3].currency_id = None;
    orders[4].province_id = ProvinceId(3);
    orders[5].quantity = f64::NAN;
    let scope = MarketScope::from_world(&w).unwrap();
    let index = OrderIndex::new(&orders, &scope);
    assert_eq!(index.raw_orders(&scope.key(ProvinceId(1)), &"flour".into()), &[0, 2, 3]);
    assert_eq!(index.food_orders(&Some("COIN".into()), &"flour".into()), &[0, 2, 4]);
    assert_eq!(index.food_orders(&None, &"flour".into()), &[3]);
    w.market_data.territory = serde_json::from_value(json!([
        {"province_id":1,"is_floating":false,"landmass_id":1,"neighbors":[3,2,{"neighbor_id":5,"contact_type":"sea"},6]},
        {"province_id":2,"is_floating":false,"landmass_id":1,"neighbor_ids":"4; 7"},
        {"province_id":3,"is_floating":false,"landmass_id":1,"neighbors":[4]},
        {"province_id":4,"is_floating":false,"landmass_id":1,"neighbors":[]},
        {"province_id":5,"is_floating":false,"landmass_id":1,"neighbors":[]},
        {"province_id":6,"is_floating":true,"landmass_id":1,"neighbors":[5]},
        {"province_id":7,"is_floating":false,"landmass_id":"1","neighbors":[]}
    ])).unwrap();
    let mut paths = LandPathIndex::new(&w.market_data, 512).unwrap();
    let expected = vec![ProvinceId(1), ProvinceId(3), ProvinceId(4)];
    assert_eq!(&*paths.path(ProvinceId(1), ProvinceId(4)).unwrap().unwrap(), &expected);
    for (from, to) in [(4,1),(1,5),(1,6),(1,7),(99,1)] {
        assert!(paths.path(ProvinceId(from), ProvinceId(to)).unwrap().is_none());
    }
    // Force eviction, including negative results; all queries retain direction and tie order.
    for n in 10..2010 {
        assert!(paths.path(ProvinceId(n), ProvinceId(4)).unwrap().is_none());
        assert!(paths.cache_bytes() <= 512);
    }
    assert_eq!(&*paths.path(ProvinceId(1), ProvinceId(4)).unwrap().unwrap(), &expected);
    let measurement = meter::Measurement::start();
    for _ in 0..1000 { assert!(paths.path(ProvinceId(1), ProvinceId(4)).unwrap().is_some()); }
    let (allocated, _) = measurement.finish();
    assert!(allocated < 4096, "repeated path allocated {allocated} bytes");

    // Graph creation copies typed topology, not arbitrary authored payload.
    let mut data = MarketData::default();
    data.territory = (1..=256).map(|n| serde_json::from_value(json!({
        "province_id":n,"is_floating":false,"landmass_id":"land",
        "neighbors":[n+1],"notes":"x".repeat(16384)
    })).unwrap()).collect();
    let measurement = meter::Measurement::start();
    let mut paths = LandPathIndex::new(&data, 512).unwrap();
    let (allocated, _) = measurement.finish();
    assert!(allocated < 256 * 1024, "graph copied authored payload: {allocated}");
    // A path larger than cache capacity is returned but never retained.
    assert_eq!(paths.path(ProvinceId(1), ProvinceId(256)).unwrap().unwrap().len(), 256);
    assert!(paths.cache_bytes() <= 512);
}
