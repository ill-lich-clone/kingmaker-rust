#[path = "common/dynamic_week.rs"]
mod dynamic;
#[path = "common/market.rs"]
mod market;
#[path = "common/production.rs"]
mod production;
use kingmaker_core::*;
use kingmaker_sim::trade_offers::plan_trade_offers;
use serde_json::json;
fn fixture() -> WorldState {
    let mut w = production::world();
    w.market_data.price_calibration = market::market_world().market_data.price_calibration;
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("steel".into(), json!(0.003));
    let mut stock = w.stockpiles[0].clone();
    stock.external_id = "real-steel".into();
    stock.id = 12;
    stock.goods = Goods::Steel;
    stock.quantity = 2.;
    stock.currency_id = Some("COIN".into());
    stock.facility_id = Some("wh".into());
    stock.inventory_origin = Some("produced".into());
    w.stockpiles.push(stock);
    w.market_data.sale_offers = serde_json::from_value(json!([
        {"id":"manual","stock_id":"other","seller_id":"author","goods_id":"iron","seller_purpose":"authored","author_formula":"keep"},
        {"id":"obsolete","stock_id":"gone","seller_id":"o","goods_id":"steel","automatic":true}
    ])).unwrap();
    w
}
#[test]
fn automatic_offers_refresh_real_output_keep_manual_fields_and_exact_fractional_price() {
    let w = fixture();
    let before = serde_json::to_value(&w).unwrap();
    let offers = plan_trade_offers(&w, "w").unwrap();
    assert_eq!(offers.len(), 2);
    assert_eq!(offers[0], w.market_data.sale_offers[0]);
    assert_eq!(offers[1].id, Some("standing-sale:real-steel".into()));
    assert_eq!(
        offers[1].seller_purpose.as_deref(),
        Some("employer_operating_capital")
    );
    assert_eq!(offers[1].extra["price_minor_per_unit"], json!("0"));
    assert_eq!(
        offers[1].extra["price_minor_rate"],
        json!({"numerator":"300","denominator":"1000"})
    );
    assert_eq!(offers[1].extra["period"], json!("w"));
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
    let mut changed = w.clone();
    changed.stockpiles.last_mut().unwrap().quantity = 0.;
    assert_eq!(
        plan_trade_offers(&changed, "w2").unwrap(),
        vec![w.market_data.sale_offers[0].clone()]
    );
}
#[test]
fn food_reserves_unproduced_pop_stock_unknown_currency_unit_and_private_inputs_are_not_offered() {
    for case in 0..6 {
        let mut w = fixture();
        match case {
            0 => w.stockpiles.last_mut().unwrap().purpose = "food_reserve".into(),
            1 => {
                let p = w.pops[0].clone();
                w.pops[0].external_id = "o".into();
                w.stockpiles.last_mut().unwrap().inventory_origin = Some("opening".into());
                assert_ne!(p.external_id, w.pops[0].external_id);
            }
            2 => w.stockpiles.last_mut().unwrap().currency_id = None,
            3 => w.stockpiles.last_mut().unwrap().unit = None,
            4 => w
                .market_data
                .price_calibration
                .as_mut()
                .unwrap()
                .prices_by_currency
                .get_mut("COIN")
                .unwrap()
                .insert("steel".into(), json!(0))
                .map(|_| ())
                .unwrap(),
            _ => {
                w.stockpiles.last_mut().unwrap().goods = Goods::Iron;
            }
        }
        assert_eq!(plan_trade_offers(&w, "w").unwrap().len(), 1, "case {case}");
    }
}
#[test]
fn produced_artisan_pop_output_and_raw_output_used_as_input_remain_sellable() {
    let mut w = fixture();
    w.pops[0].external_id = "o".into();
    let f = w.production_data.factories.remove(0);
    let mut a: kingmaker_core::production::ArtisanEnterprise =
        serde_json::from_value(serde_json::to_value(f).unwrap()).unwrap();
    a.base.method_id = "artisan:steel".into();
    w.production_data.artisans.push(a);
    let m = &mut w.production_data.catalog.as_mut().unwrap().methods[0];
    m.method_id = "artisan:steel".into();
    m.kind = "artisan".into();
    assert_eq!(
        plan_trade_offers(&w, "w").unwrap()[1]
            .seller_purpose
            .as_deref(),
        Some("operating_capital")
    );
    w.pops.clear();
    w.production_data.resource_operations.push(serde_json::from_value(json!({"id":"raw","owner_id":"o","province_id":1,"method_id":"rgo:iron","resource_id":"iron","facility_id":"wh","assignments":[]})).unwrap());
    let mut m = w.production_data.catalog.as_ref().unwrap().methods[0].clone();
    m.method_id = "rgo:iron".into();
    m.kind = "rgo".into();
    m.output_goods_id = "iron".into();
    m.inputs.clear();
    w.production_data.catalog.as_mut().unwrap().methods.push(m);
    w.stockpiles[0].currency_id = Some("COIN".into());
    w.market_data
        .price_calibration
        .as_mut()
        .unwrap()
        .prices_by_currency
        .get_mut("COIN")
        .unwrap()
        .insert("iron".into(), json!(1));
    assert!(plan_trade_offers(&w, "w")
        .unwrap()
        .iter()
        .any(|o| o.stock_id == ExternalId::from("iron")));
}
#[test]
fn public_raw_stock_requires_its_own_public_warehouse() {
    let mut w = fixture();
    w.production_data.factories.clear();
    w.stockpiles.last_mut().unwrap().owner_id = "treasury:s".into();
    assert_eq!(plan_trade_offers(&w, "w").unwrap().len(), 1);
    w.production_data.storage_facilities[0].owner_id = "treasury:s".into();
    w.production_data.storage_facilities[0]
        .extra
        .insert("state_raw_warehouse".into(), json!(true));
    assert_eq!(plan_trade_offers(&w, "w").unwrap().len(), 2);
}
#[test]
fn automatic_offers_match_actual_source_including_fractional_quote_and_unknown_manual_data() {
    let mut w = fixture();
    for quote in [0.003, 0.015, 3.] {
        w.market_data
            .price_calibration
            .as_mut()
            .unwrap()
            .prices_by_currency
            .get_mut("COIN")
            .unwrap()
            .insert("steel".into(), json!(quote));
        let expected: Vec<kingmaker_core::market::SaleOffer> =
            serde_json::from_value(dynamic::oracle("trade-offers", &w)).unwrap();
        assert_eq!(plan_trade_offers(&w, "w").unwrap(), expected);
    }
}
#[test]
fn ambiguous_generated_offer_ids_refuse_without_aliasing_numeric_and_string_stock_titles() {
    let mut w = fixture();
    let mut numeric = w.stockpiles.last().unwrap().clone();
    numeric.external_id = ExternalId::from_json(json!(7)).unwrap();
    numeric.id = 15;
    let mut string = numeric.clone();
    string.external_id = "7".into();
    string.id = 16;
    w.stockpiles.extend([numeric, string]);
    let before = serde_json::to_value(&w).unwrap();
    assert!(plan_trade_offers(&w, "w").is_err());
    assert_eq!(serde_json::to_value(&w).unwrap(), before);
}
#[test]
fn actual_native_production_becomes_sellable_without_seeded_output_or_offer() {
    let mut w=production::world();
    w.market_data.price_calibration=market::market_world().market_data.price_calibration;
    w.market_data.price_calibration.as_mut().unwrap().prices_by_currency.get_mut("COIN").unwrap().insert("steel".into(),json!(3));
    assert!(w.stockpiles.iter().all(|s|s.goods!=Goods::Steel));assert!(w.market_data.sale_offers.is_empty());
    let result=kingmaker_sim::production::execute_production_cycle(&mut w,&production::request()).unwrap();
    assert_eq!(result.outputs.iter().map(|o|o.quantity).sum::<f64>(),1.);
    let offers=plan_trade_offers(&w,"w").unwrap();assert_eq!(offers.len(),1);
    let actual=w.stockpiles.iter().find(|s|s.goods==Goods::Steel).unwrap();
    assert_eq!(offers[0].stock_id,actual.external_id);assert_eq!(actual.quantity,1.);
    assert_eq!(offers[0].extra["price_minor_per_unit"],json!("300"));
}
