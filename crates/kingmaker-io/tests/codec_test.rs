use kingmaker_io::codec::CodecContext;
use serde_json::{json, Value};
use std::collections::HashMap;
const TAG: &str = "__kingmaker_codec_v2__";
fn id(n: u64) -> Value {
    json!({TAG:"id","value":n})
}
fn context() -> CodecContext {
    CodecContext::new(
        HashMap::from([(2, json!(7)), (9, json!("7")), (20, json!("salt"))]),
        HashMap::from([(
            ("goods".to_string(), 3),
            json!({"goods_id":id(20),"authored":true}),
        )]),
    )
}
fn record(value: Value) -> Value {
    json!({"storage_schema_version":2,"table":"test","value":value,"bigint_fields":[],"extensions":{}})
}
#[test]
fn typed_dictionary_ids_remain_distinct() {
    let out = context()
        .decode_record(&record(json!({"numeric":id(2),"text":id(9)})))
        .unwrap();
    assert_eq!(out, json!({"numeric":7,"text":"7"}));
}
#[test]
fn unknown_dictionary_id_is_an_error() {
    assert!(context()
        .decode_record(&record(json!({"owner_id":id(99)})))
        .is_err());
}
#[test]
fn shared_authored_values_are_decoded_not_lost() {
    let out = context()
        .decode_record(&record(
            json!({"goods":{TAG:"shared","table":"goods","index":3}}),
        ))
        .unwrap();
    assert_eq!(out, json!({"goods":{"goods_id":"salt","authored":true}}));
}
#[test]
fn missing_shared_entry_is_an_error() {
    assert!(context()
        .decode_record(&record(json!({TAG:"shared","table":"goods","index":4})))
        .is_err());
}
#[test]
fn bigint_extensions_and_escaped_keys_keep_their_types() {
    let mut r = record(
        json!({"amount_minor":"9007199254740993","plain":"9007199254740993",
        "__kingmaker_codec_v2__~__kingmaker_codec_v2__":"literal", "existing":1}),
    );
    r["bigint_fields"] = json!(["amount_minor"]);
    r["extensions"] = json!({"existing":2,"extra":{"beneficiary_pop_id":id(9)}});
    let out = context().decode_record(&r).unwrap();
    assert_eq!(
        out,
        json!({"amount_minor":{TAG:"bigint","value":"9007199254740993"},
        "plain":"9007199254740993",TAG:"literal","existing":1,"extra":{"beneficiary_pop_id":"7"}})
    );
}
#[test]
fn bigint_path_escaping_and_shared_context_are_preserved() {
    let ctx = CodecContext::new(
        HashMap::new(),
        HashMap::from([(("lease".into(), 0), json!({"a/b":"42"}))]),
    );
    let mut r = record(json!({"lease":{TAG:"shared","table":"lease","index":0}}));
    r["bigint_fields"] = json!(["lease/a~1b"]);
    assert_eq!(
        ctx.decode_record(&r).unwrap(),
        json!({"lease":{"a/b":{TAG:"bigint","value":"42"}}})
    );
}
#[test]
fn marker_lookalike_with_authored_extra_field_is_an_ordinary_object() {
    let r = record(json!({TAG:"id","value":999,"authored":true}));
    assert_eq!(context().decode_record(&r).unwrap(), r["value"]);
}
#[test]
fn undefined_set_map_date_and_nonfinite_values_are_explicit() {
    let r = record(
        json!({"missing":{TAG:"undefined"},"set":{TAG:"set","value":[id(9)]},
        "map":{TAG:"map","value":[[id(2),id(9)]]},"date":{TAG:"date","value":"2026-10-12T00:00:00.000Z"},
        "nan":{TAG:"number","value":"NaN"}}),
    );
    assert_eq!(
        context().decode_record(&r).unwrap(),
        json!({"missing":{TAG:"undefined"},
        "set":{TAG:"set","value":["7"]},"map":{TAG:"map","value":[[7,"7"]]},
        "date":{TAG:"date","value":"2026-10-12T00:00:00.000Z"},"nan":{TAG:"number","value":"NaN"}})
    );
}
#[test]
fn malformed_version_or_marker_is_rejected() {
    let mut r = record(json!({"a":1}));
    r["storage_schema_version"] = json!(3);
    assert!(context().decode_record(&r).is_err());
    assert!(context()
        .decode_record(&record(json!({TAG:"id","value":"9"})))
        .is_err());
    assert!(context()
        .decode_record(&record(json!({TAG:"number","value":"typo"})))
        .is_err());
}
#[test]
fn codec_agrees_with_real_js_oracle_on_nested_values() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut r = record(json!({"owner_id":id(9),"amount_minor":"9007199254740993",
        "goods":{TAG:"shared","table":"goods","index":3},"missing":{TAG:"undefined"},
        "set":{TAG:"set","value":[id(2),id(9)]},"map":{TAG:"map","value":[[id(9),{TAG:"bigint","value":"42"}]]},
        "__kingmaker_codec_v2__~__kingmaker_codec_v2__":"author_value"}));
    r["bigint_fields"] = json!(["amount_minor"]);
    r["extensions"] = json!({"more":{"extra":true}});
    let input = json!({"dictionary":[[2,7],[9,"7"],[20,"salt"]],"shared":[["goods",3,{"goods_id":id(20),"authored":true}]],"record":r});
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/tests/codec-oracle.cjs");
    let mut child = Command::new("node")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "JS oracle: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let js: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(context().decode_record(&input["record"]).unwrap(), js);
}
