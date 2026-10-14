use kingmaker_core::{ExternalId, Goods, ProvinceId};
use serde_json::json;
#[test]
fn province_normalization_is_explicit_and_does_not_truncate() {
    assert_eq!(
        ProvinceId::from_external(&ExternalId::from_json(json!(1000)).unwrap())
            .unwrap()
            .0,
        1000
    );
    assert_eq!(
        ProvinceId::from_external(&ExternalId::from_json(json!("1000")).unwrap())
            .unwrap()
            .0,
        1000
    );
    assert_eq!(
        ProvinceId::from_external(&ExternalId::from_json(json!(140080)).unwrap())
            .unwrap()
            .0,
        140080
    );
    assert!(ProvinceId::from_external(
        &ExternalId::from_json(json!("holding_region_004")).unwrap()
    )
    .is_err());
    assert!(
        ProvinceId::from_external(&ExternalId::from_json(json!(4294967296u64)).unwrap()).is_err()
    );
}
#[test]
fn author_goods_keep_identity_and_never_fall_back_to_grain() {
    for code in [
        "salt",
        "clay",
        "medicines",
        "wood",
        "textiles",
        "future_authored_good",
    ] {
        let original = ExternalId::from_json(json!(code)).unwrap();
        let goods = Goods::from_external(original.clone());
        assert_ne!(goods, Goods::Grain);
        assert_eq!(goods.external_id(), original);
        assert_eq!(serde_json::to_value(goods).unwrap(), json!(code));
    }
}
#[test]
fn typed_identifiers_keep_separate_hash_keys() {
    let n = ExternalId::from_json(json!(7)).unwrap();
    let s = ExternalId::from_json(json!("7")).unwrap();
    let index = std::collections::HashMap::from([(n, 100), (s, 200)]);
    assert_eq!(index.len(), 2);
}
#[test]
fn money_json_keeps_integer_precision_for_js_consumers() {
    let amount = kingmaker_core::Money(9007199254740993);
    assert_eq!(
        serde_json::to_value(amount).unwrap(),
        json!("9007199254740993")
    );
}
