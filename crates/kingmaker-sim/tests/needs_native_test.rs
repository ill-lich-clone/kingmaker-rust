mod common {
    pub mod market;
}
use common::market::*;
use kingmaker_core::market::*;
use kingmaker_core::*;
use kingmaker_sim::market_policy::MarketScope;
use kingmaker_sim::needs::{build_need_orders, resolve_need_quantities};
use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

fn row<'a>(rows: &'a [ResolvedNeed], need: &str) -> &'a ResolvedNeed {
    rows.iter()
        .find(|r| r.need_id.as_str() == Some(need))
        .expect("authoritative need row missing")
}

#[test]
fn weekly_food_is_authored_not_point_one() {
    let (needs, reference) = need_fixture();
    let rows = resolve_need_quantities(&needs, &reference).unwrap();
    assert_eq!(row(&rows, "food").gross_quantity, Some(10.0));
    let w = market_world();
    let orders = build_need_orders(
        &w,
        &MarketProfile {
            food_norm_multiplier: Some(2.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        orders
            .orders
            .iter()
            .filter(|o| o.pop_id.as_ref() == Some(&"a".into()))
            .map(|o| o.quantity)
            .sum::<f64>(),
        20.0
    );
}

#[test]
fn weekly_norm_missing_is_error_and_not_month_divided() {
    let (mut needs, reference) = need_fixture();
    needs.profiles[0].norms[0]
        .extra
        .remove("quantity_per_person_week");
    needs.profiles[0].norms[0]
        .extra
        .insert("quantity_per_person_month".into(), json!(0.04));
    assert!(resolve_need_quantities(&needs, &reference)
        .unwrap_err()
        .to_string()
        .contains("Missing native weekly norm food"));
    needs.resolver.extra.insert("period_days".into(), json!(30));
    assert!(resolve_need_quantities(&needs, &reference)
        .unwrap_err()
        .to_string()
        .contains("Invalid weekly resolver period"));
}

#[test]
fn unknown_need_is_null_and_policy_is_not_consumed() {
    let (mut needs, reference) = need_fixture();
    needs.profiles[0].norms.extend(
        serde_json::from_value::<Vec<NeedNorm>>(json!([
            {"need_id":"housing","status":"unknown"},{"need_id":"budget","status":"policy"}
        ]))
        .unwrap(),
    );
    let rows = resolve_need_quantities(&needs, &reference).unwrap();
    assert_eq!(row(&rows, "housing").quantity, None);
    assert_eq!(
        row(&rows, "housing").extra["coverage_status"],
        json!("not_evaluated")
    );
    assert_eq!(row(&rows, "budget").extra["consumable"], json!(false));
}

#[test]
fn luxury_literacy_upgrade_and_military_match_authored_rules() {
    let (mut needs, mut reference) = need_fixture();
    reference
        .extra
        .insert("wealth_band_id".into(), json!("rich"));
    reference.extra.insert("literacy".into(), json!(0.75));
    reference
        .extra
        .insert("selected_upgrade_groups".into(), json!(["apparel"]));
    needs.resolver.extra.insert("normalizers".into(),json!({"printed_goods":{"H":2,"mean_raw":1},"clothing":{"H":2,"mean_raw":1},"luxury_clothing":{"H":1,"mean_raw":1}}));
    needs.profiles[0].norms.extend(serde_json::from_value::<Vec<NeedNorm>>(json!([
        {"need_id":"printed","goods_id":"printed_goods","allocation_rule":"wealth_weight×literacy_factor","budget_scope":"everyday"},
        {"need_id":"clothing","goods_id":"clothing","allocation_rule":"wealth_weight","functional_group":"apparel","substitution_coefficient":2,"budget_scope":"basic"},
        {"need_id":"luxury_clothing","goods_id":"luxury_clothing","allocation_rule":"wealth_weight","functional_group":"apparel","luxury":true,"budget_scope":"luxury"},
        {"need_id":"military","status":"known"}
    ])).unwrap());
    let rows = resolve_need_quantities(&needs, &reference).unwrap();
    assert_eq!(row(&rows, "printed").quantity, Some(3.75));
    assert_eq!(row(&rows, "clothing").quantity, Some(0.0));
    reference
        .extra
        .insert("active_army_workers".into(), json!(1000));
    needs.resolver.extra.insert(
        "military_links".into(),
        json!([{"input_goods_id":"grain","coefficient":1}]),
    );
    needs.resolver.extra.insert(
        "military_H".into(),
        json!({"weapons":0.02,"ammunition":0.04}),
    );
    let rows = resolve_need_quantities(&needs, &reference).unwrap();
    assert_eq!(row(&rows, "food").quantity, Some(3.0));
    assert_eq!(
        rows.iter()
            .find(|r| r.extra.get("goods_id") == Some(&json!("military_supply")))
            .unwrap()
            .quantity,
        Some(7.0)
    );
}

#[test]
fn slaves_use_agreement_and_missing_agreement_never_creates_wallet() {
    let mut w = market_world();
    w.pops[0].pop_type_id = "slaves".into();
    w.market_data.maintenance_agreements.push(
        serde_json::from_value(json!({"pop_id":"a","payer_id":"payer","currency_id":"COIN"}))
            .unwrap(),
    );
    let r = build_need_orders(&w, &MarketProfile::default()).unwrap();
    let o = r
        .orders
        .iter()
        .find(|o| o.pop_id.as_ref() == Some(&"a".into()))
        .expect("maintenance order missing");
    assert_eq!(o.owner_id, ExternalId::from("payer"));
    assert_eq!(o.purpose, "sponsored_maintenance_reserve");
    assert!(o.maintenance);
    w.market_data.maintenance_agreements.clear();
    let r = build_need_orders(&w, &MarketProfile::default()).unwrap();
    assert!(r
        .orders
        .iter()
        .all(|o| o.pop_id.as_ref() != Some(&"a".into())));
}

#[test]
fn changed_reference_region_and_pop_are_not_stale() {
    let mut w = market_world();
    let alternatives = json!([{"goods_id":"grain","share":0.5},{"goods_id":"fish","share":0.5}]);
    w.market_data.needs.as_mut().unwrap().profiles[0].norms[0]
        .extra
        .insert("alternatives".into(), alternatives);
    let before = build_need_orders(&w, &MarketProfile::default()).unwrap();
    assert_eq!(
        before
            .orders
            .iter()
            .find(|o| o.pop_id.as_ref() == Some(&"a".into()))
            .unwrap()
            .goods_id,
        ExternalId::from("grain")
    );
    w.pops[0].province_id = ProvinceId(3);
    w.market_data.needs.as_mut().unwrap().profile_refs[0]
        .extra
        .insert("size".into(), json!(500));
    w.stockpiles.push(StockpileRecord {
        id: 2,
        external_id: "fish:3".into(),
        province_id: ProvinceId(3),
        goods: Goods::Fish,
        quantity: 20.0,
        owner_id: "seller".into(),
        unit: Some(json!("t")),
        currency_id: Some("COIN".into()),
        ..Default::default()
    });
    let after = build_need_orders(&w, &MarketProfile::default()).unwrap();
    let a = after
        .orders
        .iter()
        .find(|o| o.pop_id.as_ref() == Some(&"a".into()))
        .unwrap();
    assert_eq!(a.goods_id, ExternalId::from("fish"));
    assert_eq!(a.quantity, 5.0);
    assert_eq!(a.province_id, ProvinceId(3));
}

#[test]
fn absent_needs_is_not_measured_and_invalid_multiplier_is_error() {
    let mut w = market_world();
    w.market_data.needs = None;
    let r = build_need_orders(&w, &MarketProfile::default()).unwrap();
    assert!(r.orders.is_empty());
    assert!(!r.diagnostics.is_empty());
    assert!(build_need_orders(
        &market_world(),
        &MarketProfile {
            food_norm_multiplier: Some(-1.0),
            ..Default::default()
        }
    )
    .is_err());
}

#[test]
fn resolver_complete_rows_match_the_real_js_oracle() {
    let (mut needs, mut reference) = need_fixture();
    needs.profiles[0].norms.extend(serde_json::from_value::<Vec<NeedNorm>>(json!([
        {"need_id":"heat","alternatives":[{"goods_id":"coal","H":0.025}],"status":"unresolved"},
        {"need_id":"leisure","budget_scope":"everyday","alternatives":[{"goods_id":"spirits","H":0.15,"allocation_rule":"wealth_weight"}]},
        {"need_id":"clothing","goods_id":"clothing","H":1,"allocation_rule":"none","quantity_per_person_week":0.001,"budget_scope":"basic"},
        {"need_id":"education","status":"not_applicable"},{"need_id":"wealth_bands"},{"need_id":"military"}
    ])).unwrap());
    needs
        .resolver
        .extra
        .insert("normalizers".into(), json!({"spirits":{"mean_raw":1}}));
    reference
        .extra
        .insert("active_army_workers".into(), json!(17));
    reference.extra.insert("size".into(), json!("1000,0"));
    let native =
        serde_json::to_value(resolve_need_quantities(&needs, &reference).unwrap()).unwrap();
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/market-oracle.cjs"
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
        .write_all(
            serde_json::to_string(&json!({"action":"needs","needs":needs,"reference":reference}))
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let js: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_source_values(&native, &js);
}

#[test]
fn complete_need_orders_match_js_for_food_and_nonfood() {
    let mut w = market_world();
    w.market_data.needs.as_mut().unwrap().profiles[0].norms.push(serde_json::from_value(json!({"need_id":"clothing","goods_id":"clothing","H":1,"allocation_rule":"none","quantity_per_person_week":0.001,"budget_scope":"basic"})).unwrap());
    let snapshot = json!({"registry":{"needs":w.market_data.needs,"pops":w.pops.iter().map(|p|json!({"pop_id":p.external_id,"pop_type_id":p.pop_type_id,"province_id":p.province_id.0,"currency_id":p.currency_id})).collect::<Vec<_>>(),"territory":w.market_data.territory,"foundationParameters":w.market_data.parameters,"maintenanceAgreements":w.market_data.maintenance_agreements},"stocks":w.stockpiles.iter().map(|s|json!({"province_id":s.province_id.0,"goods_id":s.goods.external_id(),"amount":s.quantity})).collect::<Vec<_>>()});
    let native = serde_json::to_value(
        build_need_orders(&w, &MarketProfile::default())
            .unwrap()
            .orders,
    )
    .unwrap();
    let mut child = Command::new("node")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../scripts/tests/market-oracle.cjs"
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
        .write_all(
            serde_json::to_string(&json!({"action":"orders","snapshot":snapshot}))
                .unwrap()
                .as_bytes(),
        )
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_source_values(
        &native,
        &serde_json::from_slice::<Value>(&out.stdout).unwrap(),
    );
}

#[test]
fn typed_regions_missing_regions_and_conflicts_preserve_scope() {
    let mut w = market_world();
    w.market_data.territory[0].region_id = Some(ExternalId::Number(7.into()));
    w.market_data.territory[1].region_id = Some(ExternalId::String("7".into()));
    let scope = MarketScope::from_world(&w).unwrap();
    assert!(!scope.same(ProvinceId(1), ProvinceId(2)));
    w.market_data.territory[0].region_id = None;
    w.market_data.territory[1].region_id = None;
    assert!(!MarketScope::from_world(&w)
        .unwrap()
        .same(ProvinceId(1), ProvinceId(2)));
    w.market_data
        .territory
        .push(serde_json::from_value(json!({"province_id":1,"region_id":"conflict"})).unwrap());
    assert!(MarketScope::from_world(&w).is_err());
}

#[test]
fn quantity_rounding_monthly_and_undefined_metadata_match_js() {
    for h in [
        5e-13,
        f64::from_bits((5e-13_f64).to_bits() + 1),
        1.0000000000005,
        -0.0000000000005,
    ] {
        let (mut needs, reference) = need_fixture();
        needs.profiles[0].norms.push(serde_json::from_value(json!({"need_id":"tools","goods_id":"tools","H":h,"allocation_rule":"wealth_weight","budget_scope":"everyday","unit":{"__kingmaker_codec_v2__":"undefined"}})).unwrap());
        needs
            .resolver
            .extra
            .insert("normalizers".into(), json!({"tools":{"mean_raw":1}}));
        let native =
            serde_json::to_value(resolve_need_quantities(&needs, &reference).unwrap()).unwrap();
        let mut child = Command::new("node")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/tests/market-oracle.cjs"
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
            .write_all(
                serde_json::to_string(
                    &json!({"action":"needs","needs":needs,"reference":reference}),
                )
                .unwrap()
                .as_bytes(),
            )
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_source_values(
            &native,
            &serde_json::from_slice::<Value>(&out.stdout).unwrap(),
        );
    }
    let (mut needs, reference) = need_fixture();
    needs.schema_version = None;
    needs.resolver.extra.insert("period".into(), json!("month"));
    needs.profiles[0].norms[0]
        .extra
        .insert("quantity_per_person_month".into(), json!(0.04));
    assert_eq!(
        row(
            &resolve_need_quantities(&needs, &reference).unwrap(),
            "food"
        )
        .quantity,
        Some(40.0)
    );
}

#[test]
fn missing_reference_and_payer_remain_visible_without_invented_orders() {
    let mut w = market_world();
    w.market_data.needs.as_mut().unwrap().profile_refs.remove(0);
    let r = build_need_orders(&w, &MarketProfile::default()).unwrap();
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d["reason"] == "need_profile_reference_absent" && d["pop_id"] == "a"));
    assert!(r
        .orders
        .iter()
        .all(|o| o.pop_id.as_ref() != Some(&"a".into())));
    let mut w = market_world();
    w.pops[0].pop_type_id = "slaves".into();
    let r = build_need_orders(&w, &MarketProfile::default()).unwrap();
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d["reason"] == "maintenance_agreement_absent" && d["pop_id"] == "a"));
}

#[test]
fn stale_reference_size_is_reported_and_authored_quantity_is_preserved() {
    let mut w = market_world();
    w.market_data.needs.as_mut().unwrap().profile_refs[0]
        .extra
        .insert("size".into(), json!(500));
    let r = build_need_orders(&w, &MarketProfile::default()).unwrap();
    assert!(r
        .diagnostics
        .iter()
        .any(|d| d["reason"] == "stale_need_profile_reference_size" && d["pop_id"] == "a"));
    assert_eq!(
        r.orders
            .iter()
            .find(|o| o.pop_id.as_ref() == Some(&"a".into()))
            .unwrap()
            .quantity,
        5.0
    );
}
