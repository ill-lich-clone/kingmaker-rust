mod common;
use common::Fixture;
use kingmaker_io::SqliteLoader;
use serde_json::json;

#[test]
fn loader_preserves_real_date_in_envelope() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(w.campaign.revision, 123);
    assert_eq!(
        (w.campaign.year, w.campaign.month, w.campaign.day),
        (307, 4, 12)
    );
}
#[test]
fn loader_resolves_dictionary_province_and_class() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(w.pops[0].province_id.0, 1000);
    assert_eq!(w.pops[0].pop_type, "artisans");
    assert_eq!(w.pops[0].culture, "culture:one");
}
#[test]
fn loader_does_not_turn_currency_into_pop_account() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let actual = w.pops[0]
        .account_id
        .expect("Existing personal savings must resolve");
    assert_eq!(actual.0, 901);
    assert!(w.accounts.iter().any(|a| a.id == actual));
}
#[test]
fn loader_uses_authored_goods_and_province_not_opaque_stock_name() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(w.stockpiles[0].province_id.0, 69);
    assert_eq!(
        serde_json::to_value(&w.stockpiles[0].goods).unwrap(),
        json!("salt")
    );
    assert_eq!(w.stockpiles[0].quantity, 12.5);
}
#[test]
fn loader_builds_pop_range_for_province() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let p = w.provinces.iter().find(|p| p.id.0 == 1000).unwrap();
    let (start, end) = p.pop_range;
    assert_eq!(end - start, 1);
    assert_eq!(w.pops[start as usize].province_id.0, 1000);
}
#[test]
fn nonexistent_campaign_is_an_error() {
    let f = Fixture::new();
    assert!(SqliteLoader::load_from_path(&f.path, "missing").is_err());
}
#[test]
fn dictionary_error_is_not_a_default_province() {
    let f = Fixture::new();
    f.sql("DELETE FROM km_dictionary WHERE n=140080;");
    assert!(SqliteLoader::load_from_path(&f.path, "fixture").is_err());
}
#[test]
fn stock_owner_unit_and_authored_extensions_are_not_dropped() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let stock = serde_json::to_value(&w.stockpiles[0]).unwrap();
    assert_eq!(stock["owner_id"], json!("holding_region_004"));
    assert_eq!(stock["unit"], json!("tonnes"));
    assert_eq!(
        serde_json::to_value(&w.provinces[0].owner_state_id).unwrap(),
        json!("state_001")
    );
}
#[test]
fn monetary_overflow_is_not_silently_turned_into_zero() {
    let f = Fixture::new();
    let conn = rusqlite::Connection::open(&f.path).unwrap();
    let text: String = conn
        .query_row(
            "SELECT payload FROM km_entity WHERE table_name='accounts' AND ordinal=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut record: serde_json::Value = serde_json::from_str(&text).unwrap();
    record["value"]["amount_minor"] = json!("9223372036854775808");
    conn.execute(
        "UPDATE km_entity SET payload=? WHERE table_name='accounts' AND ordinal=0",
        [record.to_string()],
    )
    .unwrap();
    assert!(SqliteLoader::load_from_path(&f.path, "fixture").is_err());
}
#[test]
fn absent_personal_account_does_not_create_money_or_fake_reference() {
    let f = Fixture::new();
    f.sql("DELETE FROM km_entity WHERE table_name='accounts' AND ordinal=0;");
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert!(w.pops[0].account_id.is_none());
    assert_eq!(w.accounts.len(), 1);
}
#[test]
fn original_raw_tables_and_extra_fields_are_retained() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let source = w.source.as_ref().expect("Lossless source snapshot");
    let raw: serde_json::Value = serde_json::from_str(&source.records("stocks").next().unwrap().unwrap().payload).unwrap();
    assert_eq!(raw["value"]["authored_extra"], json!({"keep":true}));
    assert_eq!(source.records("accounts").map(Result::unwrap).count(), 2);
    assert_eq!(*source.dictionary_value(176).unwrap().unwrap(), json!("COIN"));
}
#[test]
fn authored_goods_change_is_visible_on_next_load_without_runtime_alias() {
    let f = Fixture::new();
    let before = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        serde_json::to_value(&before.stockpiles[0].goods).unwrap(),
        json!("salt")
    );
    let conn = rusqlite::Connection::open(&f.path).unwrap();
    let text:String=conn.query_row("SELECT payload FROM km_entity WHERE table_name='registry/territory' AND id_key='id:141569'",[],|r|r.get(0)).unwrap();
    let mut raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    raw["value"]["goods_id"] = common::id(1002);
    conn.execute("UPDATE km_entity SET payload=? WHERE table_name='registry/territory' AND id_key='id:141569'",[raw.to_string()]).unwrap();
    let after = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        serde_json::to_value(
            &after
                .provinces
                .iter()
                .find(|p| p.id.0 == 69)
                .unwrap()
                .goods_id
        )
        .unwrap(),
        json!("clay")
    );
    assert_eq!(
        serde_json::to_value(&after.stockpiles[0].goods).unwrap(),
        json!("salt"),
        "Existing stock is not the province's resource setting"
    );
    assert_eq!(after.stockpiles[0].province_id.0, 69);
    assert_eq!(after.stockpiles[0].quantity, 12.5);
}
#[test]
fn malformed_marker_in_unported_table_is_rejected() {
    let f = Fixture::new();
    f.add_record(
        "registry/opaque",
        0,
        json!("opaque"),
        common::envelope(
            "registry/opaque",
            json!({"data":{"__kingmaker_codec_v2__":"number","value":"typo"}}),
        ),
    );
    assert!(SqliteLoader::load_from_path(&f.path, "fixture").is_err());
}
#[test]
fn own_ordinal_cannot_disagree_with_intrinsic_entity_id() {
    let f = Fixture::new();
    let conn = rusqlite::Connection::open(&f.path).unwrap();
    let text: String = conn
        .query_row(
            "SELECT payload FROM km_entity WHERE table_name='stocks'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    raw["id"] = json!(1002);
    raw["external_id"] = json!("clay");
    conn.execute(
        "UPDATE km_entity SET payload=? WHERE table_name='stocks'",
        [raw.to_string()],
    )
    .unwrap();
    assert!(SqliteLoader::load_from_path(&f.path, "fixture").is_err());
}
