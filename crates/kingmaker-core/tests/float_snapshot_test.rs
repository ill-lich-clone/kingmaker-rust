#[test]
fn economic_quantities_survive_exact_json_roundtrip() {
    for q in [
        0.44999999999999996_f64,
        0.1_f64,
        1.1_f64,
        0.49999999999999994_f64,
    ] {
        let text = serde_json::to_string(&q).unwrap();
        let restored: f64 = serde_json::from_str(&text).unwrap();
        assert_eq!(restored.to_bits(), q.to_bits(), "{text}");
    }
}
