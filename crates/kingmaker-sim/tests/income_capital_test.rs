#[path = "common/labor.rs"]
mod labor;
#[path = "common/market.rs"]
mod market;
use kingmaker_core::*;
use kingmaker_sim::income_capital::working_capital_requirements;
use serde_json::{json, Value};
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn fixture() -> WorldState {
    let mut w = market::market_world();
    w.pops[0].pop_type_id = "artisans".into();
    w.pops[0].pop_type = "artisans".into();
    w.accounts[0].balance = Money(100000);
    w.production_data.catalog.as_mut().unwrap().methods=serde_json::from_value(json!([
        {"method_id":"artisan:cloth","kind":"artisan","status":"ready","output_goods_id":"clothing","output_unit":"t","weekly_productivity":1,"staffing":["artisans"],"inputs":[{"goods_id":"grain","amount":2}]},
        {"method_id":"artisan:tools","kind":"artisan","status":"ready","output_goods_id":"tools","output_unit":"t","weekly_productivity":2,"staffing":["artisans"],"inputs":[{"goods_id":"grain","amount":2}]}
    ])).unwrap();
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("clothing".into(), json!(3));
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("tools".into(), json!(4));
    w.production_data.artisans=serde_json::from_value(json!([{"id":"art:a","owner_id":"a","province_id":1,"method_id":"artisan:cloth","payment_purpose":"operating_capital","assignments":[{"pop_id":"a","workers_count":10,"time_fraction":1}],"income_policy":{"source":"fixture","cash_reserve_minor":"500","payout_fraction":1},"working_capital":{"currency_id":"COIN","personal_funded_minor":"123"},"undistributed_profit_minor":"45"}])).unwrap();
    w
}
fn snapshot(w: &WorldState) -> Value {
    let mut s = labor::snapshot(w);
    s["campaign"] =
        json!({"date":{"year":w.campaign.year,"month":w.campaign.month,"day":w.campaign.day}});
    let r = s["registry"].as_object_mut().unwrap();
    r.insert("territory".into(), json!(w.market_data.territory));
    r.insert(
        "foundationParameters".into(),
        json!(w.market_data.parameters),
    );
    r.insert("saleOffers".into(), json!(w.market_data.sale_offers));
    r.insert("shipments".into(), json!(w.mobility_data.shipments));
    r.insert(
        "transportAssets".into(),
        json!(w.transport_data.transport_assets),
    );
    r.insert(
        "transportTerminals".into(),
        json!(w.transport_data.transport_terminals),
    );
    r.insert("supplyRoutes".into(), json!(w.transport_data.supply_routes));
    r.insert(
        "navigationGeography".into(),
        json!(w.transport_data.navigation_geography),
    );
    if let Some(policy) = &w.transport_data.automatic_land_paths {
        r.insert("automatic_land_paths".into(), policy.clone());
    }
    for (row, stock) in s["stocks"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(&w.stockpiles)
    {
        row["currency_id"] = json!(stock.currency_id);
        row["inventory_purpose"] = json!(stock.purpose);
        row["inventory_origin"] = json!(stock.inventory_origin);
        row["method_kind"] = json!(stock.method_kind);
    }
    s
}
fn oracle(w: &WorldState, quotes: &[Value]) -> Value {
    let mut p = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/income-oracle.cjs"
        ))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin
        .take()
        .unwrap()
        .write_all(
            json!({"action":"capital","snapshot":snapshot(w),"quotes":quotes})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
    let out = p.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn check(w: &WorldState, quotes: &[Value]) -> Value {
    let before = serde_json::to_value(w).unwrap();
    let actual = serde_json::to_value(working_capital_requirements(w, quotes).unwrap()).unwrap();
    let mut expected = oracle(w, quotes);
    // JS serializes integral Number as 10; serde f64 as 10.0. Quantities remain exact f64.
    for row in expected["entities"].as_array_mut().unwrap() {
        row["feasible_output"] = json!(row["feasible_output"].as_f64().unwrap());
    }
    assert_eq!(actual, expected);
    assert_eq!(serde_json::to_value(w).unwrap(), before);
    actual
}

#[test]
fn unavailable_current_or_pending_method_preserves_authored_reserve_and_group_rights() {
    for pending in [false, true] {
        let mut w = fixture();
        let position = if pending { 1 } else { 0 };
        let method = &mut w.production_data.catalog.as_mut().unwrap().methods[position];
        method.status = "unresolved".into();
        method.weekly_productivity = None;
        if pending {
            w.production_data.artisans[0].pending_retooling =
                Some(json!({"method_id":"artisan:tools"}));
        }
        let result = check(&w, &[]);
        let entity = &result["entities"][0];
        assert_eq!(entity["possible"], false);
        assert_eq!(entity["feasible_output"], 0.0);
        assert_eq!(entity["requested_minor"], "0");
        assert_eq!(entity["minimum_minor"], "500");
        assert_eq!(result["groups"][0]["minimum_minor"], "500");
        assert_eq!(result["groups"][0]["personal_funded_minor"], "123");
        assert_eq!(result["groups"][0]["retained_profit_minor"], "45");
        assert_eq!(result["groups"][0]["active"], true);
    }
}

#[test]
fn ready_method_without_productivity_still_refuses_with_enterprise_method_and_province() {
    let mut w = fixture();
    w.production_data.catalog.as_mut().unwrap().methods[0].weekly_productivity = None;
    let before = serde_json::to_value(&w).unwrap();
    let error = working_capital_requirements(&w, &[])
        .err()
        .expect("ready missing productivity must fail")
        .to_string();
    for context in ["art:a", "artisan:cloth", "province", "1"] {
        assert!(error.contains(context), "{error}");
    }
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
fn remote(w: &mut WorldState) {
    w.market_data.parameters.as_mut().unwrap().regional_market = Some(false);
    for row in &mut w.market_data.territory {
        row.extra.insert(
            "neighbor_ids".into(),
            if row.province_id == ProvinceId(1) {
                json!([2])
            } else if row.province_id == ProvinceId(2) {
                json!([1, 3])
            } else {
                json!([2])
            },
        );
    }
    w.transport_data.transport_assets = vec![
        json!({"id":"fleet","owner_id":"carrier","home_province_id":2,"modes":["land"],"unit":"space","capacity":12,"status":"source","source":"fixture"}),
    ];
    w.transport_data.supply_routes = vec![
        json!({"id":"route","from_province_id":2,"to_province_id":1,"carrier_id":"carrier","asset_id":"fleet","currency_id":"COIN","mode":"land","unit":"space","path":[2,1],"capacity_per_week":20,"source_handling_per_week":20,"destination_handling_per_week":20,"travel_days":7,"return_days":7,"freight_minor_per_space":"2","goods":{"grain":{"unit":"t","space_per_unit":1}},"status":"source","source":"fixture"}),
    ];
    w.market_data.sale_offers.push(serde_json::from_value(json!({"id":"offer","stock_id":"grain:2","seller_id":"seller","goods_id":"grain","currency_id":"COIN","unit":"t","status":"source","source":"fixture"})).unwrap());
}

#[test]
fn shared_weekly_route_and_terminal_limits_are_not_promised_twice() {
    for kind in [
        "capacity_per_week",
        "source_handling_per_week",
        "destination_handling_per_week",
    ] {
        let mut w = fixture();
        remote(&mut w);
        w.transport_data.supply_routes[0][kind] = json!(2);
        let mut other = w.production_data.artisans[0].clone();
        other.base.id = "other".into();
        w.production_data.artisans.push(other);
        let r = working_capital_requirements(&w, &[]).unwrap();
        assert_eq!(r.entities[0].feasible_output, 1.);
        assert_eq!(r.entities[1].feasible_output, 0., "shared limit {kind}");
        assert!(!r.entities[1].possible);
    }
}

#[test]
fn joint_recipe_uses_shared_fleet_and_reserves_the_feasible_input_and_freight_cost() {
    let mut w = fixture();
    remote(&mut w);
    w.production_data.catalog.as_mut().unwrap().methods[0].inputs = serde_json::from_value(
        json!([{"goods_id":"grain","amount":1},{"goods_id":"coal","amount":1}]),
    )
    .unwrap();
    let mut coal = w.stockpiles[0].clone();
    coal.id = 99;
    coal.external_id = "coal:2".into();
    coal.goods = Goods::Coal;
    w.stockpiles.push(coal);
    w.market_data.sale_offers.push(serde_json::from_value(json!({"id":"coaloffer","stock_id":"coal:2","seller_id":"seller","goods_id":"coal","currency_id":"COIN","unit":"t","status":"source","source":"fixture"})).unwrap());
    w.transport_data.supply_routes[0]["goods"]["coal"] = json!({"unit":"t","space_per_unit":1});
    let r = working_capital_requirements(&w, &[]).unwrap();
    assert!(r.entities[0].possible);
    assert_eq!(r.entities[0].feasible_output, 6.);
    assert_eq!(r.entities[0].requested_minor, Money(1224));
    assert_eq!(r.entities[0].minimum_minor, Money(1224));
}

#[test]
fn local_reserve_is_exact_and_does_not_spend_any_money_or_stock() {
    let w = fixture();
    let r = check(&w, &[]);
    assert_eq!(r["entities"][0]["requested_minor"], json!("2000"));
    assert_eq!(r["groups"][0]["minimum_minor"], json!("2000"));
    assert_eq!(r["groups"][0]["personal_funded_minor"], json!("123"));
}
#[test]
fn unused_draft_asset_with_unknown_capacity_does_not_block_local_trade() {
    let mut w = fixture();
    w.transport_data
        .transport_assets
        .push(json!({"id":"draft-fleet","status":"draft","source":"unfinished"}));
    assert_eq!(
        check(&w, &[])["entities"][0]["requested_minor"],
        json!("2000")
    );
}
#[test]
fn missing_route_vehicle_is_a_rejection_not_a_successful_validation() {
    let mut w = fixture();
    remote(&mut w);
    w.transport_data.transport_assets.clear();
    let validator = kingmaker_sim::transport_validation::RouteValidation::new(&w).unwrap();
    assert_eq!(
        validator.reason(
            &w.transport_data.supply_routes[0],
            &w.market_data.sale_offers[0]
        ),
        Some("transport_parameters_unknown")
    );
}
#[test]
fn held_owned_inputs_and_food_reserves_are_distinguished() {
    let mut w = fixture();
    let mut stock = w.stockpiles[0].clone();
    stock.id = 900;
    stock.external_id = "own".into();
    stock.owner_id = "a".into();
    stock.province_id = ProvinceId(1);
    stock.quantity = 4.;
    w.stockpiles.push(stock.clone());
    w.production_data.goods_reservations.push(
        serde_json::from_value(json!({"stock_id":"own","status":"held","quantity":2})).unwrap(),
    );
    stock.id = 901;
    stock.external_id = "food".into();
    stock.quantity = 1000.;
    stock.purpose = "food_reserve".into();
    w.stockpiles.push(stock);
    assert_eq!(
        check(&w, &[])["entities"][0]["requested_minor"],
        json!("1800")
    );
}
#[test]
fn inbound_only_counts_if_due_within_next_seven_days_and_not_storage_delayed() {
    for (day, delayed, expected) in [(8, false, true), (9, false, false), (8, true, false)] {
        let mut w = fixture();
        w.stockpiles[0].quantity = 0.;
        w.mobility_data.shipments = vec![
            json!({"id":"incoming","buyer_id":"a","to_province_id":1,"goods_id":"grain","remaining_quantity":8,"status":"in_transit","storage_delayed":delayed,"arrival_date":{"year":306,"month":1,"day":day}}),
        ];
        let result = check(&w, &[]);
        assert_eq!(result["entities"][0]["possible"], json!(expected));
        assert_eq!(result["entities"][0]["requested_minor"], json!("0"));
    }
}
#[test]
fn shared_inputs_rollback_unfundable_fraction_without_double_use() {
    let mut w = fixture();
    w.accounts[0].balance = Money(1000);
    let mut second = w.production_data.artisans[0].clone();
    second.base.id = "art:a:second".into();
    w.production_data.artisans.push(second);
    let result = check(&w, &[]);
    assert_eq!(result["entities"][0]["feasible_output"], json!(10.));
    assert_eq!(result["entities"][1]["feasible_output"], json!(5.));
    assert_eq!(result["groups"][0]["minimum_minor"], json!("3000"));
    w.accounts[0].balance = Money(0);
    check(&w, &[]);
}
#[test]
fn real_fleet_capacity_busy_cargo_route_currency_and_land_path_limit_forecast() {
    let mut w = fixture();
    remote(&mut w);
    let result = check(&w, &[]);
    assert_eq!(result["entities"][0]["feasible_output"], json!(6.));
    assert_eq!(result["entities"][0]["requested_minor"], json!("1224"));
    w.mobility_data.shipments = vec![
        json!({"id":"busy","asset_id":"fleet","remaining_quantity":1,"shipped_quantity":4,"space_per_unit":1}),
    ];
    assert_eq!(check(&w, &[])["entities"][0]["feasible_output"], json!(4.));
    w.mobility_data.shipments.clear();
    w.transport_data.supply_routes[0]["currency_id"] = json!("OTHER");
    assert_eq!(check(&w, &[])["entities"][0]["possible"], json!(false));
    w.transport_data.supply_routes[0]["currency_id"] = json!("COIN");
    w.market_data.territory[0]
        .extra
        .insert("is_floating".into(), json!(true));
    assert_eq!(check(&w, &[])["entities"][0]["possible"], json!(false));
}
#[test]
fn automatic_land_route_uses_existing_fleet_and_exact_source_edge_freight() {
    let mut w = fixture();
    remote(&mut w);
    w.transport_data.automatic_land_paths = Some(json!(true));
    w.transport_data.supply_routes[0]["to_province_id"] = json!(3);
    w.transport_data.supply_routes[0]["path"] = json!([2, 3]);
    assert_eq!(
        check(&w, &[])["entities"][0]["requested_minor"],
        json!("1224")
    );
}
#[test]
fn unoffered_stock_cannot_poison_route_cache_for_a_later_authorized_stock() {
    let mut w = fixture();
    remote(&mut w);
    let mut unoffered = w.stockpiles[0].clone();
    unoffered.id = 990;
    unoffered.external_id = "unoffered".into();
    unoffered.quantity = 5.;
    w.stockpiles.insert(0, unoffered);
    // Proven legacy cache defect: the first stock caches missing permission for all stocks.
    assert_eq!(oracle(&w, &[])["entities"][0]["possible"], json!(false));
    let result = working_capital_requirements(&w, &[]).unwrap();
    assert!(result.entities[0].possible);
    assert_eq!(result.entities[0].requested_minor, Money(1224));
}
#[test]
fn each_forecast_lot_respects_its_route_limit_and_exact_freight() {
    let mut w = fixture();
    remote(&mut w);
    w.stockpiles[0].quantity = 20.;
    w.transport_data.supply_routes[0]["capacity_per_week"] = json!(2);
    w.transport_data.transport_assets[0]["capacity"] = json!(20);
    let mut stock = w.stockpiles[0].clone();
    stock.id = 992;
    stock.external_id = "grain:3".into();
    stock.province_id = ProvinceId(3);
    w.stockpiles.push(stock);
    w.transport_data.transport_assets.push(json!({"id":"fleet:3","owner_id":"carrier:3","home_province_id":3,"modes":["land"],"unit":"space","capacity":20,"status":"source","source":"fixture"}));
    let mut route = w.transport_data.supply_routes[0].clone();
    route["id"] = json!("route:3");
    route["asset_id"] = json!("fleet:3");
    route["carrier_id"] = json!("carrier:3");
    route["from_province_id"] = json!(3);
    route["path"] = json!([3, 2, 1]);
    route["travel_days"] = json!(14);
    route["capacity_per_week"] = json!(10);
    route["freight_minor_per_space"] = json!("10");
    w.transport_data.supply_routes.push(route);
    w.market_data.sale_offers.push(serde_json::from_value(json!({"id":"offer:3","stock_id":"grain:3","seller_id":"seller","goods_id":"grain","currency_id":"COIN","unit":"t","status":"source","source":"fixture"})).unwrap());
    // Legacy recomputation forgets lot.quantity and charges all 12 units to a capacity-2 route.
    assert_eq!(
        oracle(&w, &[])["entities"][0]["requested_minor"],
        json!("1224")
    );
    let result = working_capital_requirements(&w, &[]).unwrap();
    assert_eq!(result.entities[0].feasible_output, 6.);
    assert_eq!(result.entities[0].requested_minor, Money(1304)); // 1200 goods + 2*2 + 10*10 freight.
}
#[test]
fn sea_and_air_routes_require_real_navigation_and_approved_terminals() {
    for mode in ["sea", "air"] {
        let mut w = fixture();
        remote(&mut w);
        w.transport_data.transport_assets[0]["modes"] = json!([mode]);
        w.transport_data.supply_routes[0]["mode"] = json!(mode);
        w.transport_data.supply_routes[0]["source_terminal_id"] = json!("port:2");
        w.transport_data.supply_routes[0]["destination_terminal_id"] = json!("port:1");
        let kind = if mode == "sea" {
            "sea_port"
        } else {
            "air_port"
        };
        w.transport_data.transport_terminals = vec![
            json!({"id":"port:2","province_id":2,"kind":kind,"status":"source","source":"fixture"}),
            json!({"id":"port:1","province_id":1,"kind":kind,"status":"source","source":"fixture"}),
        ];
        if mode == "sea" {
            w.transport_data.supply_routes[0]["path"] = json!([2, 3, 1]);
            w.transport_data.navigation_geography = vec![
                json!({"province_id":2,"kind":"land","is_floating":false,"neighbors":[{"neighbor_id":3,"contact_type":"coast"}]}),
                json!({"province_id":3,"kind":"sea","is_floating":false,"neighbors":[{"neighbor_id":1,"contact_type":"water"}]}),
                json!({"province_id":1,"kind":"land","is_floating":false,"neighbors":[]}),
            ];
        }
        assert_eq!(check(&w, &[])["entities"][0]["possible"], json!(true));
        w.transport_data.transport_terminals[1]["status"] = json!("draft");
        assert_eq!(check(&w, &[])["entities"][0]["possible"], json!(false));
    }
}
#[test]
fn fleet_return_date_and_two_edge_automatic_freight_are_preserved() {
    let mut w = fixture();
    remote(&mut w);
    w.mobility_data.shipments = vec![
        json!({"id":"returning","asset_id":"fleet","remaining_quantity":0,"shipped_quantity":4,"space_per_unit":1,"carrier_release_date":{"year":306,"month":1,"day":8}}),
    ];
    assert_eq!(check(&w, &[])["entities"][0]["feasible_output"], json!(4.));
    w.mobility_data.shipments[0]["carrier_release_date"] = json!({"year":306,"month":1,"day":1});
    assert_eq!(check(&w, &[])["entities"][0]["feasible_output"], json!(6.));
    w.mobility_data.shipments.clear();
    w.stockpiles[0].province_id = ProvinceId(3);
    w.transport_data.transport_assets[0]["home_province_id"] = json!(3);
    w.transport_data.supply_routes[0]["from_province_id"] = json!(3);
    w.transport_data.supply_routes[0]["to_province_id"] = json!(2);
    w.transport_data.supply_routes[0]["path"] = json!([3, 2]);
    w.transport_data.automatic_land_paths = Some(json!(true));
    assert_eq!(
        check(&w, &[])["entities"][0]["requested_minor"],
        json!("1248")
    );
}
#[test]
fn pending_method_demand_and_unresolved_quotes_keep_authored_reserve() {
    let mut w = fixture();
    w.production_data.artisans[0].pending_retooling = Some(json!({"method_id":"artisan:tools"}));
    let result = check(
        &w,
        &[json!({"province_id":1,"currency_id":"COIN","goods_id":"tools","demand":3})],
    );
    assert_eq!(result["entities"][0]["feasible_output"], json!(3.));
    assert_eq!(result["entities"][0]["requested_minor"], json!("600"));
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .remove("tools");
    assert_eq!(check(&w, &[])["entities"][0]["minimum_minor"], json!("500"));
    w.production_data.artisans[0].base.assignments.clear();
    assert_eq!(check(&w, &[])["entities"][0]["active"], json!(false));
}
#[test]
fn protected_factory_inputs_are_only_public_supply_with_a_real_offer() {
    let mut w = fixture();
    let mut method = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    method.method_id = "factory:steel".into();
    method.kind = "factory".into();
    method.output_goods_id = "steel".into();
    w.production_data
        .catalog
        .as_mut()
        .unwrap()
        .methods
        .push(method);
    w.production_data.factories=serde_json::from_value(json!([{"id":"factory","owner_id":"seller","province_id":2,"method_id":"factory:steel","status":"operating","assignments":[]}])).unwrap();
    assert_eq!(check(&w, &[])["entities"][0]["possible"], json!(false));
    w.market_data.sale_offers.push(serde_json::from_value(json!({"id":"sale","stock_id":"grain:2","seller_id":"seller","goods_id":"grain","status":"approved","source":"fixture","approval_ref":"actual"})).unwrap());
    assert_eq!(check(&w, &[])["entities"][0]["possible"], json!(true));
    w.market_data.sale_offers[0].extra.remove("approval_ref");
    assert_eq!(check(&w, &[])["entities"][0]["possible"], json!(false));
}
#[test]
fn invalid_money_and_forecast_overflow_fail_without_mutating_any_registry() {
    for case in 0..2 {
        let mut w = fixture();
        if case == 0 {
            w.production_data.artisans[0]
                .base
                .extra
                .get_mut("income_policy")
                .unwrap()["cash_reserve_minor"] = json!("-1");
        } else {
            w.market_data
                .price_calibration
                .as_mut()
                .unwrap()
                .prices_by_currency
                .get_mut("COIN")
                .unwrap()
                .insert("clothing".into(), json!(1e18));
        }
        let before = serde_json::to_value(&w).unwrap();
        assert!(working_capital_requirements(&w, &[]).is_err());
        assert_eq!(serde_json::to_value(&w).unwrap(), before);
    }
}
