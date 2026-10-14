#[path = "common/market.rs"]
mod common;
use kingmaker_core::{calendar::GameDate, market::MarketOrder, *};
use kingmaker_sim::food_supply_forecast::FoodSupplyForecast;
use serde_json::json;
fn world() -> WorldState {
    let mut w = common::market_world();
    w.stockpiles.clear();
    w
}
fn food(id: &str, owner: &str, q: f64) -> MarketOrder {
    let mut o = common::order(id, owner, "grain", q, "basic", true);
    o.need_id = Some("food".into());
    o
}
fn stock(owner: &str, q: f64) -> StockpileRecord {
    StockpileRecord {
        external_id: "owned".into(),
        owner_id: owner.into(),
        province_id: ProvinceId(1),
        quantity: q,
        ..Default::default()
    }
}
fn cargo(status: &str, day: u8, q: f64) -> serde_json::Value {
    json!({"id":"food-cargo","buyer_id":"a","to_province_id":1,"goods_id":"grain","remaining_quantity":q,"status":status,"arrival_date":{"year":306,"month":1,"day":day}})
}
#[test]
fn food_orders_cover_travel_plus_next_review_and_report_actual_prearrival_shortage() {
    let w = world();
    let o = food("weekly", "a", 7.);
    let mut f = FoodSupplyForecast::new(&w, &[o.clone()]).unwrap();
    let a = f.forecast_for(&o, 7).unwrap();
    assert_eq!(
        (
            a.target_quantity,
            a.quantity,
            a.uncovered_before_arrival,
            a.horizon_days
        ),
        (14., 14., 7., 14)
    );
    let b = f.forecast_for(&o, 14).unwrap();
    assert_eq!(
        (
            b.target_quantity,
            b.quantity,
            b.uncovered_before_arrival,
            b.horizon_days
        ),
        (21., 21., 14., 21)
    );
    assert_eq!(
        b.arrival_date,
        GameDate {
            year: 306,
            month: 1,
            day: 15
        }
    );
}
#[test]
fn held_stock_and_arrival_horizon_are_not_immediately_available_food() {
    let mut w = world();
    let o = food("weekly", "a", 7.);
    w.stockpiles.push(stock("a", 10.));
    w.production_data.goods_reservations = serde_json::from_value(
        json!([{"id":"held","stock_id":"owned","quantity":3,"status":"held"}]),
    )
    .unwrap();
    w.mobility_data.shipments = vec![cargo("in_transit", 6, 4.), cargo("in_transit", 30, 100.)];
    let n = FoodSupplyForecast::new(&w, &[o.clone()])
        .unwrap()
        .forecast_for(&o, 7)
        .unwrap();
    assert_eq!(
        (
            n.on_hand,
            n.incoming_quantity,
            n.quantity,
            n.uncovered_before_arrival
        ),
        (7., 4., 3., 0.)
    );
}
#[test]
fn delayed_cargo_offsets_horizon_but_cannot_feed_people_before_arrival() {
    let mut w = world();
    let o = food("weekly", "a", 7.);
    w.stockpiles.push(stock("a", 2.));
    w.mobility_data.shipments = vec![cargo("in_transit", 4, 6.)];
    let n = FoodSupplyForecast::new(&w, &[o.clone()])
        .unwrap()
        .forecast_for(&o, 7)
        .unwrap();
    assert_eq!(
        (n.incoming_quantity, n.quantity, n.uncovered_before_arrival),
        (6., 6., 1.)
    );
    for status in ["awaiting_storage", "partial_arrived", "in_transit"] {
        w.mobility_data.shipments = vec![cargo(status, 1, 6.)];
        let n = FoodSupplyForecast::new(&w, &[o.clone()])
            .unwrap()
            .forecast_for(&o, 7)
            .unwrap();
        assert_eq!(
            (n.incoming_quantity, n.quantity, n.uncovered_before_arrival),
            (6., 6., 5.)
        );
    }
}
#[test]
fn shared_service_food_is_distributed_by_beneficiary_need_and_specific_title() {
    let mut w = world();
    let mut a = food("service:a", "service", 7.);
    a.pop_id = Some("a".into());
    let mut b = food("service:b", "service", 14.);
    b.pop_id = Some("b".into());
    w.stockpiles.push(stock("service", 9.));
    let mut specific = stock("service", 2.);
    specific.external_id = "specific".into();
    specific.beneficiary_pop_id = Some("a".into());
    w.stockpiles.push(specific);
    let mut f = FoodSupplyForecast::new(&w, &[a.clone(), b.clone()]).unwrap();
    let x = f.forecast_for(&a, 7).unwrap();
    let y = f.forecast_for(&b, 7).unwrap();
    assert_eq!(
        (x.on_hand, x.quantity, x.uncovered_before_arrival),
        (5., 9., 2.)
    );
    assert_eq!(
        (y.on_hand, y.quantity, y.uncovered_before_arrival),
        (6., 22., 8.)
    );
}
#[test]
fn dispatch_feedback_and_duplicate_order_ids_do_not_double_the_weekly_need() {
    let w = world();
    let o = food("weekly", "a", 7.);
    let mut f = FoodSupplyForecast::new(&w, &[o.clone(), o.clone()]).unwrap();
    assert_eq!(f.forecast_for(&o, 7).unwrap().quantity, 14.);
    f.register_dispatch(
        &o,
        7.,
        GameDate {
            year: 306,
            month: 1,
            day: 8,
        },
    )
    .unwrap();
    assert_eq!(f.forecast_for(&o, 7).unwrap().quantity, 7.);
    assert_eq!(f.diagnostics()[0].ordered_quantity, 7.);
}
#[test]
fn actual_js_food_forecast_matches_shared_titles_delayed_cargo_and_dispatch_feedback() {
    use kingmaker_sim::food_supply_forecast::{FoodForecastDiagnostic, FoodForecastResult};
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut w = world();
    let mut a = food("service:a", "service", 7.);
    a.pop_id = Some("a".into());
    let mut b = food("service:b", "service", 14.);
    b.pop_id = Some("b".into());
    w.stockpiles.push(stock("service", 9.));
    w.production_data.goods_reservations = serde_json::from_value(
        json!([{"id":"held","stock_id":"owned","quantity":3,"status":"held"}]),
    )
    .unwrap();
    w.mobility_data.shipments = vec![
        json!({"id":"delayed","buyer_id":"service","to_province_id":1,"goods_id":"grain","remaining_quantity":6,"status":"awaiting_storage","arrival_date":{"year":306,"month":1,"day":1}}),
    ];
    let orders = vec![a.clone(), b.clone(), a.clone()];
    let date = GameDate {
        year: 306,
        month: 1,
        day: 8,
    };
    let snapshot = json!({"campaign":{"date":GameDate::from(&w.campaign)},"stocks":[{"opening_stock_id":"owned","owner_id":"service","province_id":1,"goods_id":"grain","amount":9}],"registry":{"goodsReservations":w.production_data.goods_reservations,"shipments":w.mobility_data.shipments}});
    let input = json!({"action":"food-forecast","snapshot":snapshot,"orders":orders,"steps":[{"order":0,"days":7,"quantity":4,"date":date},{"order":1,"days":14},{"order":0,"days":7}]});
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/shipping-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let js: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut f = FoodSupplyForecast::new(&w, &orders).unwrap();
    let first = f.forecast_for(&a, 7).unwrap();
    f.register_dispatch(&a, 4., date).unwrap();
    let actual = vec![
        first,
        f.forecast_for(&b, 14).unwrap(),
        f.forecast_for(&a, 7).unwrap(),
    ];
    let expected: Vec<FoodForecastResult> =
        serde_json::from_value(js["forecasts"].clone()).unwrap();
    assert_eq!(actual, expected);
    let diagnostics: Vec<FoodForecastDiagnostic> =
        serde_json::from_value(js["diagnostics"].clone()).unwrap();
    assert_eq!(f.diagnostics(), diagnostics);
}
#[test]
fn artisans_recipe_inputs_are_protected_before_counting_owned_food() {
    let mut w = world();
    let o = food("weekly", "a", 7.);
    w.stockpiles.push(stock("a", 10.));
    w.production_data.goods_reservations = serde_json::from_value(
        json!([{"id":"held","stock_id":"owned","quantity":3,"status":"held"}]),
    )
    .unwrap();
    w.production_data.catalog.as_mut().unwrap().methods.push(serde_json::from_value(json!({"method_id":"distiller","kind":"artisan","output_goods_id":"liquor","output_unit":"t","status":"ready","weekly_productivity":2,"inputs":[{"goods_id":"grain","amount":1}],"staffing":["artisans"]})).unwrap());
    w.production_data.artisans.push(serde_json::from_value(json!({"id":"distillery","owner_id":"a","province_id":1,"method_id":"distiller","assignments":[{"pop_id":"a","workers_count":1,"time_fraction":1}],"working_capital":{"minimum_minor":"0"}})).unwrap());
    let n = FoodSupplyForecast::new(&w, &[o.clone()])
        .unwrap()
        .forecast_for(&o, 7)
        .unwrap();
    assert_eq!(
        (n.on_hand, n.quantity, n.uncovered_before_arrival),
        (5., 9., 2.)
    );
    w.stockpiles[0].purpose = "food_reserve".into();
    let n = FoodSupplyForecast::new(&w, &[o.clone()])
        .unwrap()
        .forecast_for(&o, 7)
        .unwrap();
    assert_eq!(n.on_hand, 7.);
}
