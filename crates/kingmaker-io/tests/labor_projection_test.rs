mod common;
use common::{envelope, Fixture};
use kingmaker_io::SqliteLoader;
use serde_json::json;
#[test]
fn labor_projection_retains_dynamic_pop_and_authored_rules_without_reseeding() {
    let f = Fixture::new();
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.species_id','humans','$.value.species_labor_coefficient',1.25,'$.value.mobility_status','in_transit','$.value.literate_count',33,'$.value.author_formula','unchanged') WHERE table_name='registry/pops'");
    f.add_record("registry/institutionRules",0,json!("#global"),envelope("registry/institutionRules",json!({"school":{"places_per_level":2000,"roles":[{"role_id":"teacher","pop_type_id":"clergy","weight":1}],"author":"keep"}})));
    f.add_record(
        "registry/profiles",
        0,
        json!("s"),
        envelope("registry/profiles", json!({"state_id":"s","author":"keep"})),
    );
    f.add_record(
        "registry/catalogs",
        0,
        json!("#global"),
        envelope(
            "registry/catalogs",
            json!({"species":[{"id":"humans","coefficients":{"physical":1.25}}]}),
        ),
    );
    f.add_record(
        "registry/popMobility",
        0,
        json!("#global"),
        envelope(
            "registry/popMobility",
            json!({"unemployment":{"pop:1":[{"workers":30,"weeks":2}]},"journeys":[]}),
        ),
    );
    let before = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(before.pops[0].extra["mobility_status"], json!("in_transit"));
    assert_eq!(before.pops[0].extra["literate_count"], json!(33));
    assert_eq!(before.pops[0].extra["author_formula"], json!("unchanged"));
    assert_eq!(
        before.labor_data.species[0]["coefficients"]["physical"],
        json!(1.25)
    );
    assert_eq!(
        before.labor_data.institution_rules.as_ref().unwrap()["school"]["places_per_level"],
        json!(2000)
    );
    assert_eq!(before.labor_data.profiles[0]["state_id"], json!("s"));
    assert_eq!(
        before.labor_data.pop_mobility.as_ref().unwrap()["unemployment"]["pop:1"][0]["weeks"],
        json!(2)
    );
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.species_labor_coefficient',0.75) WHERE table_name='registry/pops'");
    let after = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert_eq!(
        after.pops[0].extra["species_labor_coefficient"],
        json!(0.75)
    );
    assert_eq!(before.pops[0].workforce, after.pops[0].workforce);
    assert_eq!(before.accounts, after.accounts);
    assert_eq!(before.stockpiles, after.stockpiles);
}
#[test]
fn missing_rule_is_absent_and_pop_author_fields_do_not_shadow_native_identity() {
    let f = Fixture::new();
    f.sql("UPDATE km_entity SET payload=json_set(payload,'$.value.external_id','author-only-extra') WHERE table_name='registry/pops'");
    let w = SqliteLoader::load_from_path(&f.path, "fixture").unwrap();
    assert!(w.labor_data.institution_rules.is_none());
    assert_eq!(w.pops[0].external_id.as_str(), Some("pop:1"));
    assert_eq!(w.pops[0].extra["external_id"], json!("author-only-extra"));
    let roundtrip: kingmaker_core::WorldState =
        serde_json::from_value(serde_json::to_value(&w).unwrap()).unwrap();
    assert_eq!(roundtrip.pops, w.pops);
}
