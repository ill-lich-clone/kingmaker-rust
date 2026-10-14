mod common;
use common::{envelope, Fixture};
use serde_json::json;
#[test]
fn consumer_destinations_are_authored_lossless_and_not_guessed_from_other_warehouses() {
    let f = Fixture::new();
    let table = "registry/consumerFacilityByProvince";
    f.add_record(
        table,
        0,
        json!("#global"),
        envelope(table, json!({"1":7,"2":"7","author_unknown":{"keep":true}})),
    );
    let mut w = kingmaker_io::SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let map = w
        .market_data
        .consumer_facility_by_province
        .as_ref()
        .unwrap();
    assert_eq!(map["1"], json!(7));
    assert_eq!(map["2"], json!("7"));
    assert_eq!(map["author_unknown"], json!({"keep":true}));
    w.market_data
        .consumer_facility_by_province
        .as_mut()
        .unwrap()["1"] = json!("changed");
    let loaded: kingmaker_core::WorldState =
        serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    assert_eq!(loaded.market_data, w.market_data);
    let empty = Fixture::new();
    assert!(
        kingmaker_io::SqliteLoader::load_from_path(&empty.path, "fixture")
            .unwrap()
            .market_data
            .consumer_facility_by_province
            .is_none()
    );
}
