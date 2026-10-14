mod common;
use common::{envelope, id, Fixture};
use kingmaker_io::SqliteLoader;
use rusqlite::{params, Connection};
use serde_json::json;

fn rejection(f: &Fixture, label: &str) -> String {
    let error = match SqliteLoader::load_from_path(&f.path, "fixture") {
        Err(error) => error,
        Ok(_) => panic!("Corrupt source must be rejected"),
    };
    let text = format!("{error:#}");
    assert!(text.contains(label), "Missing row context: {text}");
    text
}

#[test]
fn corrupt_static_json_is_rejected_with_context() {
    let f = Fixture::new();
    f.sql("UPDATE km_campaign SET static_payload='not JSON'");
    rejection(&f, "@static");
}

#[test]
fn static_envelope_and_references_are_validated() {
    for raw in [
        json!({}),
        envelope("wrong", json!({})),
        envelope("@static", json!({"reference":id(9999999)})),
        envelope(
            "@static",
            json!({"reference":{"__kingmaker_codec_v2__":"shared","table":"unit","index":99}}),
        ),
    ] {
        let f = Fixture::new();
        Connection::open(&f.path)
            .unwrap()
            .execute("UPDATE km_campaign SET static_payload=?", [raw.to_string()])
            .unwrap();
        rejection(&f, "@static");
    }
}

#[test]
fn every_unported_envelope_requires_value_and_valid_metadata() {
    for field in ["value", "bigint_fields", "extensions"] {
        let f = Fixture::new();
        let mut raw = envelope("registry/opaque", json!({"keep":true}));
        raw.as_object_mut().unwrap().remove(field);
        f.add_record("registry/opaque", 0, json!("#global"), raw);
        rejection(&f, "registry/opaque");
    }
    for (field, value) in [
        ("bigint_fields", json!({})),
        ("bigint_fields", json!([false])),
        ("extensions", json!([])),
    ] {
        let f = Fixture::new();
        let mut raw = envelope("registry/opaque", json!({}));
        raw[field] = value;
        f.add_record("registry/opaque", 0, json!("#global"), raw);
        rejection(&f, "registry/opaque");
    }
}

#[test]
fn sql_external_id_cannot_disagree_with_payload_or_be_an_object() {
    for external in [json!("different-stock"), json!({"wrong":"type"})] {
        let f = Fixture::new();
        Connection::open(&f.path)
            .unwrap()
            .execute(
                "UPDATE km_entity SET external_id=? WHERE table_name='stocks'",
                [external.to_string()],
            )
            .unwrap();
        rejection(&f, "stocks");
    }
}

#[test]
fn numeric_and_string_row_identity_are_distinct_and_global_rows_remain_valid() {
    let f = Fixture::new();
    let conn = Connection::open(&f.path).unwrap();
    conn.execute("INSERT INTO km_dictionary VALUES(7,'7'),(8,'\"7\"')", [])
        .unwrap();
    f.add("registry/opaque", 7, json!(7), json!({"id":id(7)}));
    f.add("registry/opaque", 8, json!("7"), json!({"id":id(8)}));
    f.add_record(
        "registry/scalar",
        0,
        json!("#global"),
        envelope("registry/scalar", json!(true)),
    );
    assert!(SqliteLoader::load_from_path(&f.path, "fixture").is_ok());
    conn.execute(
        "UPDATE km_entity SET external_id=? WHERE table_name='registry/opaque' AND ordinal=0",
        [json!("7").to_string()],
    )
    .unwrap();
    rejection(&f, "registry/opaque");
}

#[test]
fn intrinsic_id_cannot_disagree_with_sql_even_without_envelope_id() {
    let f = Fixture::new();
    f.add_record(
        "registry/opaque",
        0,
        json!("wrong"),
        envelope("registry/opaque", json!({"id":id(240)})),
    );
    rejection(&f, "registry/opaque");
}

#[test]
fn typed_money_failure_contains_table_and_row_address() {
    let f = Fixture::new();
    let conn = Connection::open(&f.path).unwrap();
    let text: String = conn
        .query_row(
            "SELECT payload FROM km_entity WHERE table_name='accounts' AND ordinal=0",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let mut raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    raw["value"]["amount_minor"] = json!("9223372036854775808");
    conn.execute(
        "UPDATE km_entity SET payload=? WHERE table_name='accounts' AND ordinal=0",
        params![raw.to_string()],
    )
    .unwrap();
    let error = rejection(&f, "accounts");
    assert!(error.contains("id:901"), "Missing row address: {error}");
}
#[test]
fn only_province_row_address_normalizes_decimal_strings() {
    let f = Fixture::new();
    let conn = Connection::open(&f.path).unwrap();
    conn.execute(
        "UPDATE km_entity SET external_id=? WHERE table_name='registry/territory' AND ordinal=0",
        [json!("1000").to_string()],
    )
    .unwrap();
    assert!(SqliteLoader::load_from_path(&f.path, "fixture").is_ok());
    conn.execute(
        "UPDATE km_entity SET external_id=? WHERE table_name='registry/territory' AND ordinal=0",
        [json!("1001").to_string()],
    )
    .unwrap();
    rejection(&f, "registry/territory");
}
