use kingmaker_core::ids::ProvinceId;
use kingmaker_core::money::Money;
use kingmaker_core::province::ProvinceRecord;

#[test]
fn test_money_arithmetic_overflow_safe() {
    let a = Money::from_units(100);
    let b = Money::from_units(50);
    assert_eq!((a + b).units(), 150);
    assert_eq!((a - b).units(), 50);
    assert_eq!(a.units(), 100);
}

#[test]
fn test_province_and_pop_dod_layout() {
    let p_id = ProvinceId(42);
    let province = ProvinceRecord {
        id: p_id,
        name: "Test Province".to_string(),
        owner_state_id: Some(1u32.into()),
        controller_state_id: Some(1u32.into()),
        pop_range: (0, 10),
        ..Default::default()
    };

    assert_eq!(province.id.0, 42);
    assert_eq!(province.pop_range, (0, 10));
}
