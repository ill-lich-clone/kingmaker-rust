mod common;
use common::{envelope, Fixture};
use kingmaker_io::SqliteLoader;
use serde_json::json;

#[test]
fn authored_income_and_transport_rows_preserve_distinct_ids_and_unknown_terms() {
    let f = Fixture::new();
    for (table, ordinal, id, row) in [
        (
            "producerIncome",
            0,
            json!(101),
            json!({"id":101,"owner_id":7,"currency_id":"COIN","loss_carry_minor":"999999999999","author_formula":"keep"}),
        ),
        (
            "producerIncome",
            1,
            json!("101"),
            json!({"id":"101","owner_id":"7","currency_id":"COIN","loss_carry_minor":"17"}),
        ),
        (
            "transportAssets",
            0,
            json!(7),
            json!({"id":7,"capacity":3.25,"author_note":{"keep":true}}),
        ),
        (
            "transportTerminals",
            0,
            json!("7"),
            json!({"id":"7","kind":"sea_port","author_formula":"keep"}),
        ),
        (
            "supplyRoutes",
            0,
            json!("route"),
            json!({"id":"route","asset_id":7,"travel_days":21,"permission":{"source":"authored","allowed":false}}),
        ),
        (
            "routeReservations",
            0,
            json!("reservation"),
            json!({"id":"reservation","route_id":"route","period":"prior","space":2.5}),
        ),
        (
            "navigationGeography",
            0,
            json!("geo"),
            json!({"province_id":1000,"is_floating":false,"neighbors":[{"neighbor_id":69,"contact_type":"land"}]}),
        ),
    ] {
        let name = format!("registry/{table}");
        f.add_record(&name, ordinal, id, envelope(&name, row));
    }
    f.add_record(
        "registry/automatic_land_paths",
        0,
        json!("#global"),
        envelope(
            "registry/automatic_land_paths",
            json!({"enabled":true,"source":"authored","unknown":"keep"}),
        ),
    );
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(w.income_data.producer_income[0]["owner_id"], json!(7));
    assert_eq!(w.income_data.producer_income[1]["owner_id"], json!("7"));
    assert_eq!(
        w.income_data.producer_income[0]["loss_carry_minor"],
        json!("999999999999")
    );
    assert_eq!(
        w.income_data.producer_income[0]["author_formula"],
        json!("keep")
    );
    assert_eq!(w.transport_data.transport_assets[0]["id"], json!(7));
    assert_eq!(w.transport_data.transport_terminals[0]["id"], json!("7"));
    assert_eq!(
        w.transport_data.supply_routes[0]["permission"]["allowed"],
        json!(false)
    );
    assert_eq!(w.transport_data.route_reservations[0]["space"], json!(2.5));
    assert_eq!(
        w.transport_data.navigation_geography[0]["neighbors"][0]["neighbor_id"],
        json!(69)
    );
    assert_eq!(
        w.transport_data.automatic_land_paths.as_ref().unwrap()["unknown"],
        json!("keep")
    );
    let restored: kingmaker_core::WorldState =
        serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    assert_eq!(restored.income_data, w.income_data);
    assert_eq!(restored.transport_data, w.transport_data);
}

#[test]
fn direct_author_changes_are_visible_and_native_updates_survive_roundtrip() {
    let f = Fixture::new();
    f.add_record(
        "registry/producerIncome",
        0,
        json!("income"),
        envelope(
            "registry/producerIncome",
            json!({"id":"income","loss_carry_minor":"23","unknown":"keep"}),
        ),
    );
    f.add_record(
        "registry/supplyRoutes",
        0,
        json!("route"),
        envelope(
            "registry/supplyRoutes",
            json!({"id":"route","travel_days":21,"unknown":"keep"}),
        ),
    );
    let before = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.loss_carry_minor','91') WHERE table_name='registry/producerIncome'");
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.travel_days',14) WHERE table_name='registry/supplyRoutes'");
    let mut after = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        after.income_data.producer_income[0]["loss_carry_minor"],
        json!("91")
    );
    assert_eq!(
        after.transport_data.supply_routes[0]["travel_days"],
        json!(14)
    );
    assert_eq!(after.pops, before.pops);
    assert_eq!(after.accounts, before.accounts);
    assert_eq!(after.stockpiles, before.stockpiles);
    after.income_data.producer_income[0]["loss_carry_minor"] = json!("5");
    after
        .transport_data
        .route_reservations
        .push(json!({"id":"native","space":3,"unknown":"keep"}));
    let restored: kingmaker_core::WorldState =
        serde_json::from_value(serde_json::to_value(&after).unwrap()).unwrap();
    assert_eq!(
        restored.income_data.producer_income[0]["loss_carry_minor"],
        json!("5")
    );
    assert_eq!(
        restored.transport_data.route_reservations[0]["unknown"],
        json!("keep")
    );
}

#[test]
fn absent_author_navigation_policy_is_not_invented_permission() {
    let f = Fixture::new();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert!(w.income_data.producer_income.is_empty());
    assert!(w.transport_data.automatic_land_paths.is_none());
    assert!(w.transport_data.supply_routes.is_empty());
}
