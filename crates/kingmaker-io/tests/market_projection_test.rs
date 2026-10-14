mod common;
use common::{envelope, Fixture};
use kingmaker_core::ExternalId;
use kingmaker_io::SqliteLoader;
use serde_json::json;

fn prepared() -> Fixture {
    let f = Fixture::new();
    f.add_record("registry/needs", 0, json!("#global"), envelope("registry/needs", json!({
        "schema_version":"weekly-needs-v1",
        "profileRefs":[{"pop_id":"pop:1","need_profile_id":"profile:a","size":100,"wealth_band_id":"poor","selected_upgrades":{"clothing":true},"author_extra":{"keep":true}}],
        "profiles":[{"need_profile_id":"profile:a","species_food_factor":1,"norms":[
            {"need_id":"food","quantity_per_person_week":0.01,"unit":"t","alternatives":[{"goods_id":"grain","share":1}],"provenance":{"author":"fixture"}},
            {"need_id":"housing","quantity":null,"status":"unknown"}]}],
        "resolver":{"period":"week","period_days":7,"normalizers":{"tools":{"H":2,"mean_raw":1}},"author_rule":"preserve"},
        "authored_extra":{"version":2}
    })));
    f.add_record("registry/priceCalibration",0,json!("#global"),envelope("registry/priceCalibration",json!({"pricesByCurrency":{"COIN":{"grain":1.25,"tools":null}},"authored_formula":"keep"})));
    f.add_record(
        "registry/foundationParameters",
        0,
        json!("#global"),
        envelope(
            "registry/foundationParameters",
            json!({"instant_food":true,"regional_market":true,"other_switch":false}),
        ),
    );
    f.add_record("registry/maintenanceAgreements",0,json!("maintenance:a"),envelope("registry/maintenanceAgreements",json!({"id":"maintenance:a","pop_id":"pop:1","payer_id":7,"currency_id":"COIN","source":"fixture"})));
    f.add_record("registry/saleOffers",0,json!("offer:a"),envelope("registry/saleOffers",json!({"id":"offer:a","stock_id":"stock:holding_region_004:pretend_grain","seller_id":"holding_region_004","goods_id":"salt","seller_purpose":"operating_capital","status":"source","source":"fixture","author_right":"keep"})));
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.region_id',7) WHERE table_name='registry/territory' AND id_key='id:140080'");
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.region_id','7') WHERE table_name='registry/territory' AND id_key='id:141569'");
    f
}

#[test]
fn market_projection_preserves_norms_ids_and_unknown_author_fields() {
    let f = prepared();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let m = &w.market_data;
    assert!(m.needs.is_some(), "authoritative needs were not projected");
    let needs = m.needs.as_ref().unwrap();
    assert_eq!(
        needs.profile_refs[0].pop_id,
        ExternalId::String("pop:1".into())
    );
    assert_eq!(
        needs.profiles[0].norms[0].extra["quantity_per_person_week"],
        json!(0.01)
    );
    assert_eq!(needs.profiles[0].norms[1].extra["quantity"], json!(null));
    assert_eq!(needs.resolver.extra["author_rule"], json!("preserve"));
    assert_eq!(needs.extra["authored_extra"], json!({"version":2}));
    assert_eq!(
        m.maintenance_agreements[0].payer_id,
        ExternalId::Number(7.into())
    );
    assert_eq!(m.territory[0].region_id, Some(ExternalId::Number(7.into())));
    assert_eq!(
        m.territory[1].region_id,
        Some(ExternalId::String("7".into()))
    );
    assert_eq!(
        m.price_calibration.as_ref().unwrap().prices_by_currency["COIN"]["grain"],
        json!(1.25)
    );
    assert!(m.price_calibration.as_ref().unwrap().prices_by_currency["COIN"]["tools"].is_null());
    assert_eq!(m.sale_offers[0].extra["author_right"], json!("keep"));
    assert_eq!(
        m.parameters.as_ref().unwrap().extra["other_switch"],
        json!(false)
    );
}

#[test]
fn absent_needs_is_not_an_invented_zero_norm() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert!(w.market_data.needs.is_none());
    assert!(w.market_data.price_calibration.is_none());
    assert_eq!(w.market_data.territory.len(), 2);
}

#[test]
fn a_changed_norm_or_quote_is_reprojected_without_runtime_alias() {
    let f = prepared();
    let before = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.profiles[0].norms[0].quantity_per_person_week',0.025) WHERE table_name='registry/needs'");
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.pricesByCurrency.COIN.grain',2.5) WHERE table_name='registry/priceCalibration'");
    let after = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        before.market_data.needs.unwrap().profiles[0].norms[0].extra["quantity_per_person_week"],
        json!(0.01)
    );
    assert_eq!(
        after.market_data.needs.unwrap().profiles[0].norms[0].extra["quantity_per_person_week"],
        json!(0.025)
    );
    assert_eq!(
        after
            .market_data
            .price_calibration
            .unwrap()
            .prices_by_currency["COIN"]["grain"],
        json!(2.5)
    );
}

#[test]
fn singleton_null_and_undefined_are_absent_and_multiple_rows_are_rejected() {
    let f = Fixture::new();
    f.add_record(
        "registry/needs",
        0,
        json!("#global"),
        envelope("registry/needs", json!(null)),
    );
    f.add_record(
        "registry/foundationParameters",
        0,
        json!("#global"),
        envelope(
            "registry/foundationParameters",
            json!({"__kingmaker_codec_v2__":"undefined"}),
        ),
    );
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert!(w.market_data.needs.is_none());
    assert!(w.market_data.parameters.is_none());
    f.add_record(
        "registry/needs",
        1,
        json!("#other"),
        envelope("registry/needs", json!({})),
    );
    let error = SqliteLoader::load_from_path(&f.path, "fixture")
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("multiple authoritative singleton"));
}
