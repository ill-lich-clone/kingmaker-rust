mod common;
use common::{envelope, Fixture};
use kingmaker_io::SqliteLoader;
use serde_json::{json, Value};

fn add_global(f: &Fixture, table: &str, value: Value) {
    f.add_record(table, 0, json!("#global"), envelope(table, value));
}
fn prepared() -> Fixture {
    let f = Fixture::new();
    add_global(
        &f,
        "registry/productionCatalog",
        json!({"version":"production-foundation-v1","goods":[
        {"goods_id":"iron","kind":"raw_resource"},{"goods_id":"coal","kind":"raw_resource"},{"goods_id":"steel","kind":"processed_good"}],
        "methods":[{"method_id":"rgo:iron","kind":"rgo","status":"ready","output_goods_id":"iron","output_unit":"t","weekly_productivity":2,"staffing":["laborers"],"inputs":[],"technology":[]},
        {"method_id":"factory:steel","kind":"factory","status":"ready","output_goods_id":"steel","output_unit":"t","weekly_productivity":1,"staffing":["craftsmen","clerks"],"inputs":[{"goods_id":"iron","amount":1.1},{"goods_id":"coal","amount":0.4}],"technology":["tech:steel"]}],"authored_extra":{"keep":true}}),
    );
    add_global(&f, "registry/resourceOperations", json!({})); // replaced below: actual registry table stores one producer per row.
    f.sql("DELETE FROM km_entity WHERE table_name='registry/resourceOperations'");
    f.add_record("registry/resourceOperations",0,json!("rgo:1"),envelope("registry/resourceOperations",json!({"id":"rgo:1","owner_id":"holding","province_id":"1000","method_id":"rgo:iron","facility_id":"warehouse","resource_id":"iron","resource_status":"approved","approval_ref":"author","land_access":true,"initial_multiplier":3,"assignments":[{"pop_id":"pop:1","workers_count":20,"time_fraction":0.5,"employment_id":"e:1","contract_id":"c:1"}],"author_parameter":{"version":2}})));
    f.sql("INSERT INTO km_dictionary VALUES(5000,'\"f:1\"')");
    f.add("registry/factoryEnterprises",5000,json!("f:1"),json!({"id":common::id(5000),"owner_id":"holding","province_id":1000,"region_id":"region:1","method_id":"factory:steel","facility_id":"warehouse","status":"operating","people_slots":40000,"capacity_source":"author","staff_count":20,"tenure_days":6,"equipment_capacity_weekly":4,"equipment_source":"author","parameters":{"source":"author","status":"source","clerk_Bmax":0.2,"clerk_ratio":0.1,"tenure_Bmax":0.3,"tenure_Tsat":30},"assignments":[],"implemented_technologies":["tech:steel"]}));
    f.add_record("registry/contracts",0,json!("c:1"),envelope("registry/contracts",json!({"contract_id":"c:1","employer_entity_id":"holding","currency_id":"COIN","weekly_wage_rate":{"numerator":"5","denominator":"2"},"guaranteed_idle_weekly_rate":{"numerator":"1","denominator":"2"},"payment_purpose":"employer_operating_capital","expectedPayroll":{"tax_rate":0.1}})));
    f.add_record("registry/storageFacilities",0,json!("warehouse"),envelope("registry/storageFacilities",json!({"id":"warehouse","owner_id":"holding","province_id":1000,"capacity":50,"unit":"space","status":"approved","source":"author","approval_ref":"author","goods":{"steel":{"unit":"t","space_per_unit":2}},"access_contracts":[{"owner_id":"renter","status":"source","source":"author","price_minor_per_space":"7","currency_id":"COIN","purpose":"operating_capital","cancellation_policy":"refund_unused"}]})));
    f.add_record("registry/storageReservations",0,json!("held:1"),envelope("registry/storageReservations",json!({"id":"held:1","owner_id":"renter","facility_id":"warehouse","goods_id":"steel","unit":"t","status":"held","quantity":5,"accepted_quantity":8,"fee_paid_minor":{"__kingmaker_codec_v2__":"bigint","value":"112"},"lease_terms":{"currency_id":"COIN","purpose":"operating_capital","cancellation_policy":"refund_unused"}})));
    f.add_record("obligations",0,json!("debt:1"),envelope("obligations",json!({"obligation_id":"debt:1","payer_id":"holding","payee_id":"pop:1","currency_id":"COIN","purpose":"employer_operating_capital","amount_minor":"19","tax_rate":0.1,"tax_owner_id":"treasury:s"})));
    f
}

#[test]
fn production_projection_reads_authored_terms() {
    let f = prepared();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let p = &w.production_data;
    assert_eq!(p.catalog.as_ref().unwrap().methods[1].inputs[0].amount, 1.1);
    assert_eq!(p.resource_operations[0].base.province_id.0, 1000);
    assert_eq!(p.resource_operations[0].initial_multiplier, Some(3.0));
    assert_eq!(
        p.resource_operations[0].base.assignments[0].time_fraction,
        0.5
    );
    assert_eq!(p.factories[0].tenure_days, Some(6.0));
    assert_eq!(
        p.factories[0].parameters.as_ref().unwrap().clerk_bmax,
        Some(0.2)
    );
    assert_eq!(
        p.contracts[0]
            .weekly_wage_rate
            .as_ref()
            .unwrap()
            .denominator,
        "2"
    );
    assert_eq!(
        p.storage_reservations[0].fee_paid_minor.unwrap().units(),
        112
    );
    assert_eq!(p.obligations[0].amount_minor.units(), 19);
    assert_eq!(
        p.catalog.as_ref().unwrap().extra["authored_extra"],
        json!({"keep":true})
    );
    assert_eq!(
        p.resource_operations[0].base.extra["author_parameter"],
        json!({"version":2})
    );
}

#[test]
fn missing_terms_remain_absent_and_malformed_terms_are_errors() {
    let f = prepared();
    let conn = rusqlite::Connection::open(&f.path).unwrap();
    let text: String = conn
        .query_row(
            "SELECT payload FROM km_entity WHERE table_name='registry/factoryEnterprises'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut raw: Value = serde_json::from_str(&text).unwrap();
    raw["value"]
        .as_object_mut()
        .unwrap()
        .remove("equipment_capacity_weekly");
    conn.execute(
        "UPDATE km_entity SET payload=? WHERE table_name='registry/factoryEnterprises'",
        [raw.to_string()],
    )
    .unwrap();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert!(w.production_data.factories[0]
        .equipment_capacity_weekly
        .is_none());
    raw["value"]["people_slots"] = json!("broken");
    conn.execute(
        "UPDATE km_entity SET payload=? WHERE table_name='registry/factoryEnterprises'",
        [raw.to_string()],
    )
    .unwrap();
    let err = SqliteLoader::load_from_path(&f.path, "fixture")
        .err()
        .unwrap();
    assert!(format!("{err:#}").contains("registry/factoryEnterprises"));
}

#[test]
fn authored_terms_reload_without_runtime_alias() {
    let f = prepared();
    let before = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let conn = rusqlite::Connection::open(&f.path).unwrap();
    let text: String = conn
        .query_row(
            "SELECT payload FROM km_entity WHERE table_name='registry/productionCatalog'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut raw: Value = serde_json::from_str(&text).unwrap();
    raw["value"]["methods"][1]["inputs"][0]["amount"] = json!(1.2);
    conn.execute(
        "UPDATE km_entity SET payload=? WHERE table_name='registry/productionCatalog'",
        [raw.to_string()],
    )
    .unwrap();
    let after = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        after.production_data.catalog.as_ref().unwrap().methods[1].inputs[0].amount,
        1.2
    );
    assert_eq!(
        after.stockpiles, before.stockpiles,
        "Changing a recipe cannot rewrite stock goods"
    );
    assert_eq!(
        after.production_data.resource_operations[0].base.extra,
        before.production_data.resource_operations[0].base.extra
    );
}

#[test]
fn committed_event_lookup_is_read_only_and_revision_pinned() {
    let f = Fixture::new();
    f.sql("CREATE TABLE km_event(campaign TEXT,id_key TEXT,external_id TEXT,revision INTEGER,payload TEXT)");
    let c = rusqlite::Connection::open(&f.path).unwrap();
    c.execute("INSERT INTO km_event VALUES(?1,?2,?3,123,?4)",rusqlite::params![common::key(),"[\"string\",\"paid:1\"]",json!("paid:1").to_string(),envelope("@event",json!({"event_id":"paid:1","type":"payroll","period":"w","employment_id":"e","actual_fte":1})).to_string()]).unwrap();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let source = w.source.as_ref().unwrap();
    let events =
        kingmaker_io::read_committed_operations(source, &["paid:1".into(), "missing".into()])
            .unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].value["actual_fte"], json!(1));
    f.sql("UPDATE km_campaign SET revision=124");
    assert!(kingmaker_io::read_committed_operations(source, &["paid:1".into()]).is_err());
}
#[test]
fn derived_enterprise_view_updates_from_current_authoritative_terms() {
    let f = prepared();
    let mut w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    let original_id = w.enterprises[0].id;
    w.production_data.factories[0].staff_count = Some(12.0);
    w.production_data.factories[0]
        .base
        .extra
        .insert("level".into(), json!(2));
    w.refresh_enterprise_views().unwrap();
    assert_eq!(w.enterprises[0].id, original_id);
    assert_eq!(w.enterprises[0].employees, Some(12.0));
    assert_eq!(w.enterprises[0].level, 2);
}
#[test]
fn undefined_known_parameter_is_absent_and_unknown_field_keeps_its_marker() {
    let f = prepared();
    let c = rusqlite::Connection::open(&f.path).unwrap();
    let text: String = c
        .query_row(
            "SELECT payload FROM km_entity WHERE table_name='registry/factoryEnterprises'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut raw: Value = serde_json::from_str(&text).unwrap();
    let marker = json!({"__kingmaker_codec_v2__":"undefined"});
    raw["value"]["equipment_capacity_weekly"] = marker.clone();
    raw["value"]["extra_unknown"] = marker.clone();
    c.execute(
        "UPDATE km_entity SET payload=? WHERE table_name='registry/factoryEnterprises'",
        [raw.to_string()],
    )
    .unwrap();
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert!(w.production_data.factories[0]
        .equipment_capacity_weekly
        .is_none());
    assert_eq!(
        w.production_data.factories[0].base.extra["extra_unknown"],
        marker
    );
}
